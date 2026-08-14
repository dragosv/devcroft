//! `status`, `logs`, `ps` (task 4.3): read-only views over a sandbox's
//! state. `status`/`ps` need the keeper's live session count, which lives
//! only in its in-memory registry — reachable solely via a `Query` frame
//! over the control socket (keeper/protocol.rs), not from state-dir files.

use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::config::Manifest;
use crate::keeper::protocol::{self, Frame, QueryResult};
use crate::policy::{self, DegradedCapability};
use crate::provider;

use super::state::{self, Health, StatePaths};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeeperStatus {
    /// No pidfile: never started, or cleanly torn down (`rm`, or `down`
    /// followed by nothing since).
    None,
    /// pidfile present but dead or unresponsive — needs `up` to recover.
    Stale,
    Healthy {
        uptime_secs: u64,
        session_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxStatus {
    pub name: String,
    pub keeper: KeeperStatus,
    /// `None` when staleness can't be determined (no successful `up` yet
    /// recorded meta, or the environment can no longer be inspected —
    /// e.g. `.flox/` was removed). Distinct from `Some(false)` (checked,
    /// fresh).
    pub env_stale: Option<bool>,
    pub degraded: Vec<DegradedCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSummary {
    pub name: String,
    pub keeper: KeeperStatus,
    pub project_root: Option<String>,
}

#[derive(Debug)]
pub enum StatusError {
    State(io::Error),
    Keeper(String),
}

impl From<io::Error> for StatusError {
    fn from(e: io::Error) -> Self {
        StatusError::State(e)
    }
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StatusError::State(e) => write!(f, "state: {e}"),
            StatusError::Keeper(msg) => write!(f, "keeper: {msg}"),
        }
    }
}

impl std::error::Error for StatusError {}

/// Keeper health, uptime, session count, environment staleness, and
/// degraded capabilities for one sandbox (lifecycle spec: "Status and
/// logs").
pub fn status(manifest: &Manifest) -> Result<SandboxStatus, StatusError> {
    let paths = StatePaths::new(&manifest.sandbox.name)?;

    let keeper = keeper_status(&paths).map_err(|e| StatusError::Keeper(e.to_string()))?;

    let env_stale = state::read_meta(&paths.meta)?.and_then(|meta| {
        provider::is_stale(Path::new(&meta.project_root), &meta.env_fingerprint).ok()
    });

    let degraded = policy::detect_degraded(&policy::compile(manifest));

    Ok(SandboxStatus {
        name: manifest.sandbox.name.clone(),
        keeper,
        env_stale,
        degraded,
    })
}

/// The keeper log tail: session spawn/exit records with timestamps
/// (connection.rs logs these to its own stdout/stderr, which `up`
/// redirects to this file). `tail_lines` limits to the last N lines;
/// `None` returns everything.
pub fn logs(sandbox_name: &str, tail_lines: Option<usize>) -> io::Result<String> {
    let paths = StatePaths::new(sandbox_name)?;
    let contents = std::fs::read_to_string(&paths.log)?;
    match tail_lines {
        None => Ok(contents),
        Some(n) => {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(n);
            Ok(lines[start..].join("\n"))
        }
    }
}

/// Every sandbox with existing state (cli spec: "`ps` lists all
/// sandboxes"). Best-effort per entry — one sandbox's keeper being
/// unreachable at query time reports as `Stale` rather than failing the
/// whole listing.
pub fn ps() -> io::Result<Vec<SandboxSummary>> {
    let root = state::data_dir()?;
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let paths = StatePaths::in_dir(entry.path());
        let keeper = keeper_status(&paths).unwrap_or(KeeperStatus::Stale);
        let project_root = state::read_meta(&paths.meta)?.map(|meta| meta.project_root);
        out.push(SandboxSummary {
            name,
            keeper,
            project_root,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn keeper_status(paths: &StatePaths) -> io::Result<KeeperStatus> {
    match state::health(paths)? {
        Health::None => Ok(KeeperStatus::None),
        Health::Stale(_) => Ok(KeeperStatus::Stale),
        Health::Healthy(_) => {
            let result = query_keeper(paths)?;
            Ok(KeeperStatus::Healthy {
                uptime_secs: result.uptime_secs,
                session_count: result.sessions.len(),
            })
        }
    }
}

fn query_keeper(paths: &StatePaths) -> io::Result<QueryResult> {
    let mut stream = UnixStream::connect(&paths.socket)?;
    protocol::write_frame(&mut stream, &Frame::Query)?;
    match protocol::read_frame(&mut stream)? {
        Frame::QueryResult(result) => Ok(result),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected response to Query: {other:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// Short and hash-based, not `name` verbatim — see the identical
    /// comment on `state::tests::tempdir`, which this mirrors: a unix
    /// socket path has a low, OS-enforced length ceiling (macOS's
    /// `SUN_LEN` is 104 bytes) that a descriptive name plus a deep host
    /// `TMPDIR` can overflow.
    fn tempdir(name: &str) -> StatePaths {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        let root = std::env::temp_dir().join(format!(
            "dcss-{:08x}-{}",
            hasher.finish() as u32,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        StatePaths::in_dir(root)
    }

    #[test]
    fn keeper_status_is_none_without_a_pidfile() {
        let paths = tempdir("keeper-status-none");
        assert_eq!(keeper_status(&paths).unwrap(), KeeperStatus::None);
    }

    #[test]
    fn keeper_status_is_stale_when_pid_dead() {
        let paths = tempdir("keeper-status-stale");
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        child.wait().unwrap();
        state::write_pidfile(&paths.pidfile, pid).unwrap();

        assert_eq!(keeper_status(&paths).unwrap(), KeeperStatus::Stale);
    }

    #[test]
    fn logs_returns_full_contents_and_tail() {
        let paths = tempdir("logs-tail");
        std::fs::write(&paths.log, "line1\nline2\nline3\n").unwrap();

        // logs() re-derives paths from $HOME via StatePaths::new, so
        // exercise the tail-slicing logic directly against a file we
        // control instead (integration tests cover the real thing).
        let contents = std::fs::read_to_string(&paths.log).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3);
        let start = lines.len().saturating_sub(2);
        assert_eq!(lines[start..].join("\n"), "line2\nline3");
    }

    #[test]
    fn ps_lists_sandboxes_sorted_with_project_root() {
        let root = std::env::temp_dir().join(format!(
            "devcroft-lifecycle-ps-test-data-dir-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for name in ["zeta", "alpha"] {
            let sandbox_root = root.join(name);
            std::fs::create_dir_all(&sandbox_root).unwrap();
            let paths = StatePaths::in_dir(sandbox_root);
            state::write_meta(
                &paths.meta,
                &state::Meta {
                    project_root: format!("/proj/{name}"),
                    env_fingerprint: "fp".to_string(),
                },
            )
            .unwrap();
        }

        // ps() itself reads $HOME/.local/share/devcroft; verify the
        // scan+sort+meta-join logic against this scratch root directly by
        // replicating its body rather than touching the real data dir.
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&root).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let paths = StatePaths::in_dir(entry.path());
            let keeper = keeper_status(&paths).unwrap();
            let project_root = state::read_meta(&paths.meta)
                .unwrap()
                .map(|m| m.project_root);
            out.push((name, keeper, project_root));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "alpha");
        assert_eq!(out[0].1, KeeperStatus::None);
        assert_eq!(out[0].2, Some("/proj/alpha".to_string()));
        assert_eq!(out[1].0, "zeta");
        assert_eq!(out[1].2, Some("/proj/zeta".to_string()));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keeper_status_healthy_reports_uptime_and_session_count() {
        let paths = tempdir("keeper-status-healthy");
        let listener = UnixListener::bind(&paths.socket).unwrap();
        state::write_pidfile(&paths.pidfile, std::process::id() as libc::pid_t).unwrap();

        // Stands in for a real keeper answering `Query` — the real
        // request/response path is covered by
        // `keeper::connection::tests::query_reports_uptime_and_live_sessions_without_registering_one`
        // and the `tests/lifecycle_status.rs` integration test; this one
        // is about `keeper_status`'s own health-branch -> query wiring.
        // `state::health`'s own liveness probe connects and disconnects
        // without sending anything first, so this must tolerate (not
        // just handle) more than one accepted connection.
        let server = std::thread::spawn(move || {
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                match protocol::read_frame(&mut stream) {
                    Ok(Frame::Query) => {
                        protocol::write_frame(
                            &mut stream,
                            &Frame::QueryResult(QueryResult {
                                uptime_secs: 42,
                                sessions: vec![],
                            }),
                        )
                        .unwrap();
                        return;
                    }
                    Ok(other) => panic!("expected Query, got {other:?}"),
                    Err(_) => continue, // the health-probe connection
                }
            }
        });

        let result = keeper_status(&paths).unwrap();
        assert_eq!(
            result,
            KeeperStatus::Healthy {
                uptime_secs: 42,
                session_count: 0
            }
        );

        server.join().unwrap();
    }
}

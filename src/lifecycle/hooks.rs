//! Hooks (lifecycle spec's "Hooks run inside the boundary" requirement):
//! `hooks.post_create` runs once, as the first session after the sandbox's
//! first successful `up` (or after `--recreate`); `hooks.post_start` runs
//! on every keeper start. Both run exactly the way any other session does
//! — spawned over the keeper's own control socket, subject to the full
//! filesystem/network policy — never with provisioning privileges:
//! CLAUDE.md's two-phase execution model draws the line at hooks being
//! project code, not trusted host-side provisioning, even though `up`
//! itself (the caller of [`run`]) is unsandboxed host-side code.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::config::Hooks;
use crate::keeper::protocol::{self, ExitStatus, Frame, SpawnRequest};

use super::state::StatePaths;

#[derive(Debug)]
pub enum HookError {
    Connect(io::Error),
    Protocol(io::Error),
    /// The hook ran and exited non-zero or died by signal — named so
    /// `up`'s error names which hook failed, per the spec's "the error
    /// names the hook" scenario.
    Failed {
        name: &'static str,
        detail: String,
    },
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookError::Connect(e) => write!(f, "connecting to keeper: {e}"),
            HookError::Protocol(e) => write!(f, "protocol error: {e}"),
            HookError::Failed { name, detail } => write!(f, "hook `{name}` failed: {detail}"),
        }
    }
}

impl std::error::Error for HookError {}

/// Runs `hooks.post_create` (only when `run_post_create` — the caller
/// decides that from the `up` outcome: true for `Started`/`Recreated`,
/// false for `Recovered`, never called at all for `AlreadyUp`) and then
/// `hooks.post_start`, in that order. Either being unset in the manifest
/// is a no-op. Stops at the first failure rather than running both
/// unconditionally, since a failed `post_create` makes `post_start`
/// running against a half-provisioned project meaningless.
pub fn run(
    paths: &StatePaths,
    project_root: &Path,
    hooks: &Hooks,
    run_post_create: bool,
) -> Result<(), HookError> {
    if run_post_create && let Some(cmd) = &hooks.post_create {
        run_one("post_create", cmd, paths, project_root)?;
    }
    if let Some(cmd) = &hooks.post_start {
        run_one("post_start", cmd, paths, project_root)?;
    }
    Ok(())
}

/// Spawns `cmd` as a plain `sh -c` session (no pty — hooks are
/// unattended, like `post_create`'s canonical `cargo fetch` example) and
/// appends its combined stdout+stderr to `paths.log` as it streams, so
/// the spec's "hook output SHALL appear in `logs`" holds the same way a
/// keeper-logged spawn/exit record does — `devcroft logs` tails the same
/// file the keeper's own connection handler already logs to.
fn run_one(
    name: &'static str,
    cmd: &str,
    paths: &StatePaths,
    project_root: &Path,
) -> Result<(), HookError> {
    let mut stream = UnixStream::connect(&paths.socket).map_err(HookError::Connect)?;
    protocol::write_frame(
        &mut stream,
        &Frame::Spawn(SpawnRequest {
            cmd: "sh".to_string(),
            args: vec!["-c".to_string(), cmd.to_string()],
            cwd: project_root.to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            pty: None,
        }),
    )
    .map_err(HookError::Protocol)?;

    match protocol::read_frame(&mut stream).map_err(HookError::Protocol)? {
        Frame::SpawnOk { .. } => {}
        Frame::SpawnErr { message } => {
            return Err(HookError::Failed {
                name,
                detail: message,
            });
        }
        other => {
            return Err(HookError::Protocol(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected SpawnOk, got {other:?}"),
            )));
        }
    }

    let mut log = std::fs::OpenOptions::new()
        .append(true)
        .open(&paths.log)
        .map_err(HookError::Connect)?;
    // One write, for the same reason `keeper::connection::log_record`
    // exists: a `File` is unbuffered, so `writeln!` would emit one write
    // per format fragment, and the keeper is appending its own records to
    // this file concurrently. Formatting first keeps the record whole.
    let _ = log.write_all(format!("[hook {name}] $ {cmd}\n").as_bytes());

    let status: ExitStatus = loop {
        match protocol::read_frame(&mut stream).map_err(HookError::Protocol)? {
            Frame::Stdout(bytes) | Frame::Stderr(bytes) => {
                let _ = log.write_all(&bytes);
            }
            Frame::Exit(status) => break status,
            _ => {} // no other frame is expected once a session is running
        }
    };

    match status {
        ExitStatus {
            code: Some(0),
            signal: None,
        } => Ok(()),
        ExitStatus {
            code: Some(code), ..
        } => Err(HookError::Failed {
            name,
            detail: format!("exited with status {code}"),
        }),
        ExitStatus {
            signal: Some(sig), ..
        } => Err(HookError::Failed {
            name,
            detail: format!("killed by signal {sig}"),
        }),
        ExitStatus {
            code: None,
            signal: None,
        } => Err(HookError::Failed {
            name,
            detail: "exited with unknown status".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keeper::{Keeper, LocalSessionBackend};
    use std::sync::Arc;
    use std::thread;

    fn tempdir(tag: &str) -> StatePaths {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tag.hash(&mut hasher);
        let dir = std::env::temp_dir().join(format!(
            "dchk-{:08x}-{}",
            hasher.finish() as u32,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        StatePaths::in_dir(dir)
    }

    /// Against a real (in-process, unrestricted) `Keeper` — same pattern
    /// `exec.rs`'s own protocol test uses — not a real keeper process
    /// under nono; see `tests/lifecycle_up.rs` for that boundary.
    fn serve(paths: &StatePaths) {
        std::fs::File::create(&paths.log).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&paths.socket).unwrap();
        thread::spawn(move || {
            let _ = Keeper::new(listener, Arc::new(LocalSessionBackend)).serve();
        });
    }

    #[test]
    fn post_create_and_post_start_both_run_and_log_output() {
        let paths = tempdir("both-run");
        serve(&paths);

        let hooks = Hooks {
            post_create: Some("echo from-post-create".to_string()),
            post_start: Some("echo from-post-start".to_string()),
        };
        run(&paths, &paths.root, &hooks, true).unwrap();

        let log = std::fs::read_to_string(&paths.log).unwrap();
        assert!(log.contains("from-post-create"));
        assert!(log.contains("from-post-start"));
    }

    #[test]
    fn post_create_is_skipped_when_run_post_create_is_false() {
        let paths = tempdir("skip-post-create");
        serve(&paths);

        let hooks = Hooks {
            post_create: Some("echo should-not-run".to_string()),
            post_start: Some("echo should-run".to_string()),
        };
        run(&paths, &paths.root, &hooks, false).unwrap();

        let log = std::fs::read_to_string(&paths.log).unwrap();
        assert!(!log.contains("should-not-run"));
        assert!(log.contains("should-run"));
    }

    #[test]
    fn unset_hooks_are_a_no_op() {
        let paths = tempdir("no-op");
        serve(&paths);

        run(&paths, &paths.root, &Hooks::default(), true).unwrap();

        let log = std::fs::read_to_string(&paths.log).unwrap();
        assert_eq!(log, "");
    }

    #[test]
    fn a_failing_hook_names_itself_and_stops_before_post_start() {
        let paths = tempdir("failing-hook");
        serve(&paths);

        let hooks = Hooks {
            post_create: Some("exit 7".to_string()),
            post_start: Some("echo should-not-run".to_string()),
        };
        let err = run(&paths, &paths.root, &hooks, true).unwrap_err();
        assert!(matches!(
            err,
            HookError::Failed {
                name: "post_create",
                ..
            }
        ));
        assert!(err.to_string().contains("post_create"));

        let log = std::fs::read_to_string(&paths.log).unwrap();
        assert!(!log.contains("should-not-run"));
    }
}

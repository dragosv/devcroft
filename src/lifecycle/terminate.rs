//! `down` and `rm` (task 4.2): stop a sandbox's keeper, optionally
//! removing its state entirely. Both terminate the same way — SIGTERM,
//! escalating to SIGKILL after a grace period — the lifecycle spec's
//! teardown requirement names explicitly.

use std::fmt;
use std::io;
use std::path::Path;
use std::time::Duration;

use super::state::{self, Health, StatePaths};

/// Longer than the keeper's own inner per-session grace period
/// (`keeper::connection::DEFAULT_GRACE_PERIOD`, 2s): the keeper's SIGTERM
/// handler drains its sessions gracefully before exiting, and that needs
/// room to finish before this supervisor-level grace period gives up and
/// escalates to SIGKILL against the keeper process itself.
pub const GRACE_PERIOD: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum TerminateError {
    State(io::Error),
}

impl From<io::Error> for TerminateError {
    fn from(e: io::Error) -> Self {
        TerminateError::State(e)
    }
}

impl fmt::Display for TerminateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerminateError::State(e) => write!(f, "state: {e}"),
        }
    }
}

impl std::error::Error for TerminateError {}

/// Stops the keeper but keeps state and the compiled policy — a
/// subsequent `up` reuses them.
pub fn down(sandbox_name: &str) -> Result<(), TerminateError> {
    down_at(&StatePaths::new(sandbox_name)?)
}

/// Stops the keeper and removes all state for the sandbox — including
/// the service artifacts, which are the one part that does not live in
/// the state dir.
///
/// They cannot: `process-compose` has to read its config and bind its
/// socket from inside the sandbox, and the state dir is baseline-denied
/// there (`services::ARTIFACT_DIR`). So they sit in the project root,
/// and `rm` is the only thing that can clean them up. Scoped to exactly
/// `<root>/.devcroft/<name>/` — never the shared `.devcroft/` parent,
/// which may hold another sandbox's artifacts.
pub fn rm(sandbox_name: &str) -> Result<(), TerminateError> {
    let paths = StatePaths::new(sandbox_name)?;
    // Read before the state dir goes away: `meta.json` is the only
    // record of which project root this sandbox belongs to.
    let project_root = state::read_meta(&paths.meta)
        .ok()
        .flatten()
        .map(|meta| meta.project_root);
    rm_at(&paths)?;
    if let Some(root) = project_root {
        let artifacts = crate::services::artifact_dir(Path::new(&root), sandbox_name);
        match std::fs::remove_dir_all(&artifacts) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        // Best-effort: succeeds only while empty, so the last sandbox
        // out takes the shared parent with it and any other sandbox's
        // artifacts keep it alive.
        let _ = std::fs::remove_dir(Path::new(&root).join(crate::services::ARTIFACT_DIR));
    }
    Ok(())
}

// Split from `down`/`rm` so tests can drive a `StatePaths` pointed at a
// scratch dir directly, rather than mutating the process-wide `HOME` env
// var to redirect `StatePaths::new` — which previously raced every other
// test in the binary that shells out (e.g. `policy::why`'s `nono why`
// subprocess, which inherits whatever `HOME` happened to be set to at
// that instant).
fn down_at(paths: &StatePaths) -> Result<(), TerminateError> {
    stop_if_running(paths)?;
    state::clear_runtime_state(paths)?;
    Ok(())
}

fn rm_at(paths: &StatePaths) -> Result<(), TerminateError> {
    stop_if_running(paths)?;
    match std::fs::remove_dir_all(&paths.root) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn stop_if_running(paths: &StatePaths) -> io::Result<()> {
    if let Health::Healthy(pid) | Health::Stale(pid) = state::health(paths)? {
        state::terminate_and_wait(pid, GRACE_PERIOD);
    }
    // The egress proxy (add-egress-proxy) is a separate process from the
    // keeper — see `crate::proxy`'s module doc — so tearing down the
    // keeper above says nothing about it. It has no control socket to
    // probe health through the way the keeper does; its pidfile alone is
    // the only record of it, so a stale (already-dead) pid here is just
    // a no-op signal, not a distinct state worth telling apart from
    // "healthy" the way `Health` does for the keeper.
    if let Some(pid) = state::read_pidfile(&paths.proxy_pidfile)? {
        state::terminate_and_wait(pid, GRACE_PERIOD);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

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
            "dctm-{:08x}-{}",
            hasher.finish() as u32,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        StatePaths::in_dir(root)
    }

    #[test]
    fn down_is_a_no_op_when_nothing_is_running() {
        down_at(&tempdir("down-noop")).unwrap();
    }

    #[test]
    fn rm_removes_state_dir_even_when_nothing_is_running() {
        let paths = tempdir("rm-noop");
        std::fs::write(&paths.profile, "{}").unwrap();

        rm_at(&paths).unwrap();

        assert!(!paths.root.exists());
    }

    #[test]
    fn down_terminates_a_live_keeper_and_keeps_state() {
        let paths = tempdir("down-live");
        std::fs::write(&paths.profile, "{}").unwrap();
        let _listener = UnixListener::bind(&paths.socket).unwrap();

        let mut child = std::process::Command::new("sleep")
            .arg("100")
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        state::write_pidfile(&paths.pidfile, pid).unwrap();
        let reaper = std::thread::spawn(move || {
            let _ = child.wait();
        });

        down_at(&paths).unwrap();
        reaper.join().unwrap();

        assert!(!state::is_process_alive(pid));
        assert!(!paths.pidfile.exists());
        assert!(!paths.socket.exists());
        assert!(paths.profile.exists(), "down must keep compiled policy");
    }

    #[test]
    fn rm_terminates_a_live_keeper_and_removes_all_state() {
        let paths = tempdir("rm-live");
        std::fs::write(&paths.profile, "{}").unwrap();
        let _listener = UnixListener::bind(&paths.socket).unwrap();

        let mut child = std::process::Command::new("sleep")
            .arg("100")
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        state::write_pidfile(&paths.pidfile, pid).unwrap();
        let reaper = std::thread::spawn(move || {
            let _ = child.wait();
        });

        let root: PathBuf = paths.root.clone();
        rm_at(&paths).unwrap();
        reaper.join().unwrap();

        assert!(!state::is_process_alive(pid));
        assert!(!root.exists());
    }
}

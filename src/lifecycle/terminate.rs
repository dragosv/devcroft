//! `down` and `rm` (task 4.2): stop a sandbox's keeper, optionally
//! removing its state entirely. Both terminate the same way — SIGTERM,
//! escalating to SIGKILL after a grace period — the lifecycle spec's
//! teardown requirement names explicitly.

use std::fmt;
use std::io;
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

/// Stops the keeper and removes all state for the sandbox.
pub fn rm(sandbox_name: &str) -> Result<(), TerminateError> {
    rm_at(&StatePaths::new(sandbox_name)?)
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    fn tempdir(name: &str) -> StatePaths {
        let root = std::env::temp_dir().join(format!(
            "devcroft-lifecycle-terminate-test-{name}-{}",
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

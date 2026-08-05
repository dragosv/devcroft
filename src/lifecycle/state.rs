//! State-dir layout and pid/health bookkeeping for one sandbox (task 4.2).
//! Everything here is pure filesystem/process bookkeeping — `up.rs` is
//! where it gets composed into the actual supervisor sequence.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where one sandbox's runtime state lives. Mirrors the layout CLAUDE.md
/// and design.md already document as `<state>/<name>/...`.
pub struct StatePaths {
    pub root: PathBuf,
    pub socket: PathBuf,
    pub pidfile: PathBuf,
    pub profile: PathBuf,
    pub log: PathBuf,
}

impl StatePaths {
    pub fn new(sandbox_name: &str) -> io::Result<Self> {
        let root = data_dir()?.join(sandbox_name);
        Ok(StatePaths {
            socket: root.join("control.sock"),
            pidfile: root.join("keeper.pid"),
            profile: root.join("profile.json"),
            log: root.join("keeper.log"),
            root,
        })
    }
}

fn data_dir() -> io::Result<PathBuf> {
    let home = std::env::var("HOME")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".local/share/devcroft"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// No pidfile: never started, or already cleanly torn down.
    None,
    /// pidfile names a live process and its control socket accepts
    /// connections.
    Healthy(libc::pid_t),
    /// pidfile present but the process is dead, or alive yet unresponsive
    /// (socket refuses connections) — recovery (clear runtime state, then
    /// start fresh) is needed before a plain `up` can proceed.
    Stale(libc::pid_t),
}

pub fn read_pidfile(path: &Path) -> io::Result<Option<libc::pid_t>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s.trim().parse().ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn write_pidfile(path: &Path, pid: libc::pid_t) -> io::Result<()> {
    std::fs::write(path, pid.to_string())
}

/// `kill(pid, 0)`: sends no signal, just checks whether the pid could be
/// signaled. `EPERM` still means a live process (just not one we own);
/// only `ESRCH` (and friends) means it's actually gone.
pub fn is_process_alive(pid: libc::pid_t) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A pidfile alone only proves a process with that pid exists somewhere —
/// pids get reused, so liveness is confirmed by also probing the control
/// socket the same keeper would be listening on.
pub fn health(paths: &StatePaths) -> io::Result<Health> {
    let Some(pid) = read_pidfile(&paths.pidfile)? else {
        return Ok(Health::None);
    };
    if !is_process_alive(pid) {
        return Ok(Health::Stale(pid));
    }
    match UnixStream::connect(&paths.socket) {
        Ok(_) => Ok(Health::Healthy(pid)),
        Err(_) => Ok(Health::Stale(pid)),
    }
}

/// Clears everything a dead or unresponsive keeper left behind so a fresh
/// `up` can bind the socket again. Deliberately leaves `profile`/`log`
/// alone: `up` recompiles and overwrites the profile unconditionally on
/// every run, and the log is append-worthy history, not runtime state.
pub fn clear_runtime_state(paths: &StatePaths) -> io::Result<()> {
    let _ = std::fs::remove_file(&paths.pidfile);
    let _ = std::fs::remove_file(&paths.socket);
    Ok(())
}

/// SIGTERM, then SIGKILL if `pid` is still alive after `grace`. Used both
/// by `up --recreate` (replacing a running keeper) and by `down`/`rm`
/// (lifecycle::terminate) — the exact "escalating SIGTERM to SIGKILL
/// after a grace period" the lifecycle spec's teardown requirement names.
pub fn terminate_and_wait(pid: libc::pid_t, grace: Duration) {
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !is_process_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if is_process_alive(pid) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-lifecycle-state-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pidfile_roundtrips() {
        let dir = tempdir("pidfile-roundtrip");
        let path = dir.join("keeper.pid");
        assert_eq!(read_pidfile(&path).unwrap(), None);

        write_pidfile(&path, 4242).unwrap();
        assert_eq!(read_pidfile(&path).unwrap(), Some(4242));
    }

    #[test]
    fn is_process_alive_true_for_self_false_after_child_exits() {
        assert!(is_process_alive(std::process::id() as libc::pid_t));

        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        child.wait().unwrap();
        assert!(!is_process_alive(pid));
    }

    #[test]
    fn health_is_none_without_a_pidfile() {
        let dir = tempdir("health-none");
        let paths = StatePaths {
            socket: dir.join("control.sock"),
            pidfile: dir.join("keeper.pid"),
            profile: dir.join("profile.json"),
            log: dir.join("keeper.log"),
            root: dir,
        };
        assert_eq!(health(&paths).unwrap(), Health::None);
    }

    #[test]
    fn health_is_stale_when_pid_is_dead() {
        let dir = tempdir("health-stale-dead-pid");
        let paths = StatePaths {
            socket: dir.join("control.sock"),
            pidfile: dir.join("keeper.pid"),
            profile: dir.join("profile.json"),
            log: dir.join("keeper.log"),
            root: dir,
        };
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = child.id() as libc::pid_t;
        child.wait().unwrap();
        write_pidfile(&paths.pidfile, dead_pid).unwrap();

        assert_eq!(health(&paths).unwrap(), Health::Stale(dead_pid));
    }

    #[test]
    fn health_is_stale_when_pid_alive_but_socket_unresponsive() {
        let dir = tempdir("health-stale-orphan-socket");
        let paths = StatePaths {
            socket: dir.join("control.sock"), // nothing ever binds this
            pidfile: dir.join("keeper.pid"),
            profile: dir.join("profile.json"),
            log: dir.join("keeper.log"),
            root: dir,
        };
        write_pidfile(&paths.pidfile, std::process::id() as libc::pid_t).unwrap();

        assert_eq!(
            health(&paths).unwrap(),
            Health::Stale(std::process::id() as libc::pid_t)
        );
    }

    #[test]
    fn health_is_healthy_when_pid_alive_and_socket_accepts() {
        let dir = tempdir("health-healthy");
        let paths = StatePaths {
            socket: dir.join("control.sock"),
            pidfile: dir.join("keeper.pid"),
            profile: dir.join("profile.json"),
            log: dir.join("keeper.log"),
            root: dir,
        };
        let _listener = UnixListener::bind(&paths.socket).unwrap();
        write_pidfile(&paths.pidfile, std::process::id() as libc::pid_t).unwrap();

        assert_eq!(
            health(&paths).unwrap(),
            Health::Healthy(std::process::id() as libc::pid_t)
        );
    }

    #[test]
    fn clear_runtime_state_removes_pidfile_and_socket_but_keeps_profile() {
        let dir = tempdir("clear-runtime-state");
        let paths = StatePaths {
            socket: dir.join("control.sock"),
            pidfile: dir.join("keeper.pid"),
            profile: dir.join("profile.json"),
            log: dir.join("keeper.log"),
            root: dir,
        };
        write_pidfile(&paths.pidfile, 1234).unwrap();
        let _listener = UnixListener::bind(&paths.socket).unwrap();
        std::fs::write(&paths.profile, "{}").unwrap();

        clear_runtime_state(&paths).unwrap();

        assert!(!paths.pidfile.exists());
        assert!(!paths.socket.exists());
        assert!(paths.profile.exists());
    }

    #[test]
    fn terminate_and_wait_kills_a_live_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("100")
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        assert!(is_process_alive(pid));

        // A signaled process is a zombie (kill(pid, 0) still "succeeds")
        // until something reaps it; here that's this thread, standing in
        // for init reparenting + reaping a detached keeper in production
        // (`up` never waits on the keeper it spawns either — see up.rs).
        let reaper = std::thread::spawn(move || {
            let _ = child.wait();
        });

        terminate_and_wait(pid, Duration::from_secs(2));
        reaper.join().unwrap();

        assert!(!is_process_alive(pid));
    }
}

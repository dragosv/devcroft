//! State-dir layout and pid/health bookkeeping for one sandbox (task 4.2).
//! Everything here is pure filesystem/process bookkeeping — `up.rs` is
//! where it gets composed into the actual supervisor sequence.

use serde::{Deserialize, Serialize};
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
    pub meta: PathBuf,
    /// The ssh spec's embedded server socket: mode 0600, inside this
    /// (mode 0700 — see [`Self::new`]) state dir. Bound host-side and fd-
    /// inherited by the keeper, same as `socket` above — never read back
    /// off disk by the keeper itself.
    pub ssh_socket: PathBuf,
    /// The ssh spec's per-sandbox *ephemeral* host key: regenerated every
    /// `up`, never reused across them. Written here for at-rest storage,
    /// but — like every other file in this baseline-denied tree (see
    /// `policy::DEVCROFT_DATA_DIR`) — the keeper cannot read it back
    /// either; `up` passes the key material down directly instead (see
    /// `ssh::keys` and `up.rs`).
    pub ssh_host_key: PathBuf,
}

impl StatePaths {
    pub fn new(sandbox_name: &str) -> io::Result<Self> {
        Ok(Self::in_dir(data_dir()?.join(sandbox_name)))
    }

    /// Builds every path under a given root. `new` is the production
    /// entrypoint (root derived from `$HOME`); tests use this directly to
    /// point at a scratch dir without needing a struct literal repeated
    /// per file or touching the real `HOME`-derived data dir.
    pub fn in_dir(root: PathBuf) -> Self {
        StatePaths {
            socket: root.join("control.sock"),
            pidfile: root.join("keeper.pid"),
            profile: root.join("profile.json"),
            log: root.join("keeper.log"),
            meta: root.join("meta.json"),
            ssh_socket: root.join("ssh.sock"),
            ssh_host_key: root.join("ssh_host_ed25519_key"),
            root,
        }
    }
}

/// Where the client ed25519 keypair lives (ssh spec's "Key management"
/// requirement): a sibling of every sandbox's own state dir, under the
/// same data dir, rather than inside any one of them — the same keypair
/// authenticates to every sandbox, so it isn't owned by one.
pub fn client_key_paths() -> io::Result<(PathBuf, PathBuf)> {
    let dir = data_dir()?;
    Ok((dir.join("id_ed25519"), dir.join("id_ed25519.pub")))
}

/// The `~/.local/share/devcroft` root all sandboxes live under.
/// `pub(super)` so `ps` (status.rs) can enumerate every sandbox directory
/// — the one thing that needs the root itself rather than one sandbox's
/// path under it.
pub(super) fn data_dir() -> io::Result<PathBuf> {
    let home = std::env::var("HOME")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".local/share/devcroft"))
}

/// Recorded at `up`, alongside the compiled profile: what `status`/`ps`
/// (task 4.3) need but can't ask the keeper for — the project root (the
/// keeper itself is never told its own state dir) and the environment
/// fingerprint from that `up`, for `provider::is_stale` to compare
/// against the environment's current fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub project_root: String,
    pub env_fingerprint: String,
}

pub fn write_meta(path: &Path, meta: &Meta) -> io::Result<()> {
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

pub fn read_meta(path: &Path) -> io::Result<Option<Meta>> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
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

    fn tempdir(name: &str) -> StatePaths {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-lifecycle-state-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        StatePaths::in_dir(dir)
    }

    #[test]
    fn pidfile_roundtrips() {
        let paths = tempdir("pidfile-roundtrip");
        assert_eq!(read_pidfile(&paths.pidfile).unwrap(), None);

        write_pidfile(&paths.pidfile, 4242).unwrap();
        assert_eq!(read_pidfile(&paths.pidfile).unwrap(), Some(4242));
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
    fn meta_roundtrips() {
        let paths = tempdir("meta-roundtrip");
        assert_eq!(read_meta(&paths.meta).unwrap(), None);

        let meta = Meta {
            project_root: "/proj".to_string(),
            env_fingerprint: "abc123".to_string(),
        };
        write_meta(&paths.meta, &meta).unwrap();
        assert_eq!(read_meta(&paths.meta).unwrap(), Some(meta));
    }

    #[test]
    fn health_is_none_without_a_pidfile() {
        let paths = tempdir("health-none");
        assert_eq!(health(&paths).unwrap(), Health::None);
    }

    #[test]
    fn health_is_stale_when_pid_is_dead() {
        let paths = tempdir("health-stale-dead-pid");
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = child.id() as libc::pid_t;
        child.wait().unwrap();
        write_pidfile(&paths.pidfile, dead_pid).unwrap();

        assert_eq!(health(&paths).unwrap(), Health::Stale(dead_pid));
    }

    #[test]
    fn health_is_stale_when_pid_alive_but_socket_unresponsive() {
        let paths = tempdir("health-stale-orphan-socket"); // socket never bound
        write_pidfile(&paths.pidfile, std::process::id() as libc::pid_t).unwrap();

        assert_eq!(
            health(&paths).unwrap(),
            Health::Stale(std::process::id() as libc::pid_t)
        );
    }

    #[test]
    fn health_is_healthy_when_pid_alive_and_socket_accepts() {
        let paths = tempdir("health-healthy");
        let _listener = UnixListener::bind(&paths.socket).unwrap();
        write_pidfile(&paths.pidfile, std::process::id() as libc::pid_t).unwrap();

        assert_eq!(
            health(&paths).unwrap(),
            Health::Healthy(std::process::id() as libc::pid_t)
        );
    }

    #[test]
    fn clear_runtime_state_removes_pidfile_and_socket_but_keeps_profile() {
        let paths = tempdir("clear-runtime-state");
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

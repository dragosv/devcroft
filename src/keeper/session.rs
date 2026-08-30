//! Spawns the child process behind a session, in either mode a
//! `SpawnRequest` can ask for (task 4.1). Both modes make the child its
//! own process-group leader so `kill(-pgid, sig)` (registry.rs) reaches
//! the whole job without any risk of hitting the keeper itself.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use super::protocol::{PtySize, SpawnRequest};
use super::pty;

pub struct SpawnedSession {
    pub child: Child,
    pub pgid: libc::pid_t,
    pub stdin: Option<Box<dyn Write + Send>>,
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Option<Box<dyn Read + Send>>,
    /// `Some` only for pty sessions; `connection::handle` uses it to
    /// service `Resize` frames.
    pub resize_handle: Option<File>,
}

/// How a session actually comes into being. The `process` tier's keeper
/// (and the ssh server, whichever tier it runs under) drive every session
/// through this trait rather than calling `spawn` directly, so a
/// hardened backend with a native exec-into primitive (e.g. `runsc exec`)
/// can supply its own implementation — in its own module — without
/// `connection.rs`/`pty.rs`/`protocol.rs`/`registry.rs` or the ssh
/// channel handling ever needing to know which one is in play. Session
/// semantics (exec, shell, pty, signals, exit codes) come along for free
/// because none of that code touches the spawn mechanism directly.
pub trait SessionBackend: Send + Sync {
    fn spawn(&self, req: &SpawnRequest) -> io::Result<SpawnedSession>;
}

/// Local fork/exec — today's only backend, and the `process` tier's.
pub struct LocalSessionBackend;

impl SessionBackend for LocalSessionBackend {
    fn spawn(&self, req: &SpawnRequest) -> io::Result<SpawnedSession> {
        spawn(req)
    }
}

pub fn spawn(req: &SpawnRequest) -> io::Result<SpawnedSession> {
    match &req.pty {
        Some(size) => spawn_pty(req, size),
        None => spawn_piped(req),
    }
}

/// Resets SIGINT/SIGQUIT/SIGTERM/SIGHUP to `SIG_DFL` *and* unblocks them,
/// the same thing every job-control shell does before exec'ing a
/// foreground command. Without this, whatever the keeper itself
/// inherited — e.g. `SIG_IGN` disposition, or the signals sitting in its
/// *blocked* mask — would leak into every spawned session. Both are
/// inherited across fork/exec independently, and either one alone is
/// enough to silently swallow forwarded signals the client explicitly
/// asked to send (connection.rs's `Frame::Signal` -> `kill(-pgid, sig)`):
/// a process with the disposition ignored just has the kernel drop the
/// signal before delivery, and a process with it blocked has the kernel
/// mark it pending forever since ordinary programs like `sleep` never
/// unblock signals themselves — either way the process runs to
/// completion oblivious to it. A non-interactive ancestor shell (CI
/// runner, editor task, container entrypoint) commonly leaves both in
/// this state for exactly this signal set.
///
/// SAFETY: `signal(3)` and `pthread_sigmask(3)` are async-signal-safe and
/// only touch this (freshly forked, single-threaded, about to exec)
/// child's own disposition table and mask.
unsafe fn reset_forwarded_signal_dispositions() {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGHUP] {
            libc::signal(sig, libc::SIG_DFL);
            libc::sigaddset(&mut set, sig);
        }
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}

fn spawn_piped(req: &SpawnRequest) -> io::Result<SpawnedSession> {
    let mut cmd = Command::new(&req.cmd);
    cmd.args(&req.args)
        .current_dir(&req.cwd)
        .envs(&req.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setpgid(0, 0) and signal(2, SIG_DFL) are async-signal-safe
    // and touch only the child's own (freshly forked, single-threaded)
    // process state.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            reset_forwarded_signal_dispositions();
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;
    let pgid = child.id() as libc::pid_t;
    let stdin = child
        .stdin
        .take()
        .map(|s| Box::new(s) as Box<dyn Write + Send>);
    let stdout = Box::new(child.stdout.take().expect("stdout piped")) as Box<dyn Read + Send>;
    let stderr = child
        .stderr
        .take()
        .map(|s| Box::new(s) as Box<dyn Read + Send>);

    Ok(SpawnedSession {
        child,
        pgid,
        stdin,
        stdout,
        stderr,
        resize_handle: None,
    })
}

fn spawn_pty(req: &SpawnRequest, size: &PtySize) -> io::Result<SpawnedSession> {
    let opened = pty::open_pty(size)?;
    let stdout_handle = opened.master.try_clone()?;
    let resize_handle = opened.master.try_clone()?;
    let slave_fd = opened.slave_fd;

    let mut cmd = Command::new(&req.cmd);
    cmd.args(&req.args)
        .current_dir(&req.cwd)
        .envs(&req.env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid/ioctl/dup2/close below are all async-signal-safe and
    // operate only on fds already owned by this (freshly forked,
    // single-threaded) child. Making the child a session leader and
    // handing it the slave as its controlling terminal is exactly what a
    // terminal emulator does for any interactive shell.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::dup2(slave_fd, 0) < 0
                || libc::dup2(slave_fd, 1) < 0
                || libc::dup2(slave_fd, 2) < 0
            {
                return Err(io::Error::last_os_error());
            }
            if slave_fd > 2 {
                libc::close(slave_fd);
            }
            reset_forwarded_signal_dispositions();
            Ok(())
        });
    }

    let spawn_result = cmd.spawn();
    // The parent's copy of the slave fd is never needed again: the child
    // has its own dup2'd copies on 0/1/2 (or spawning failed entirely).
    unsafe {
        libc::close(slave_fd);
    }
    let child = spawn_result?;
    let pgid = child.id() as libc::pid_t;

    Ok(SpawnedSession {
        child,
        pgid,
        stdin: Some(Box::new(opened.master) as Box<dyn Write + Send>),
        stdout: Box::new(stdout_handle) as Box<dyn Read + Send>,
        stderr: None,
        resize_handle: Some(resize_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn req(cmd: &str, args: &[&str], pty: Option<PtySize>) -> SpawnRequest {
        SpawnRequest {
            cmd: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: "/".to_string(),
            env: BTreeMap::new(),
            pty,
        }
    }

    #[test]
    fn piped_session_echoes_stdin_to_stdout_and_exits_cleanly() {
        let mut session = spawn(&req("cat", &[], None)).unwrap();
        session.stdin.take().unwrap().write_all(b"hello\n").unwrap();
        // stdin was owned by a Box that's now dropped, closing the pipe so
        // `cat` sees EOF and exits.

        let mut out = Vec::new();
        session.stdout.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"hello\n");

        let status = session.child.wait().unwrap();
        assert_eq!(status.code(), Some(0));
    }

    #[test]
    fn piped_session_is_its_own_process_group() {
        let session = spawn(&req("sh", &["-c", "echo $$"], None)).unwrap();
        // setpgid(0,0) makes the child its own group leader, so pgid == pid.
        assert_eq!(session.pgid, session.child.id() as libc::pid_t);
    }

    #[test]
    fn pty_session_produces_output_over_the_master() {
        let mut session = spawn(&req(
            "sh",
            &["-c", "echo hello-pty"],
            Some(PtySize { rows: 24, cols: 80 }),
        ))
        .unwrap();
        assert!(session.resize_handle.is_some());

        let out = read_until_closed(&mut session.stdout);
        assert!(
            String::from_utf8_lossy(&out).contains("hello-pty"),
            "expected pty output to contain the echoed text, got {out:?}"
        );
    }

    /// Once a pty's last slave closes, Linux/BSD return `EIO` on further
    /// master reads instead of a clean `Ok(0)` — connection.rs's `pump`
    /// treats that the same as EOF; tests need the same tolerance or a
    /// process that exits before the buffer drains loses data to
    /// `read_to_end`'s default "discard on error" behavior.
    fn read_until_closed(r: &mut dyn Read) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                Err(e) => panic!("unexpected read error: {e}"),
            }
        }
        out
    }
}

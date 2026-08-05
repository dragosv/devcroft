//! The `exec` capability (task 5.1): client-side one-shot execution
//! against a running keeper's control socket. `shell` (task 5.2) reuses
//! this connect/stream/signal machinery for pty sessions.
//!
//! This is the client-side mirror of `keeper::connection`: where the
//! keeper owns the child and relays its stdio as frames, this owns the
//! local process's real stdio and relays *that* as frames, forwarding
//! SIGINT/SIGTERM/SIGHUP so `exec -- sleep 100` behaves like running
//! `sleep 100` directly (exec spec: "Ctrl-C reaches the child").

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;

use crate::keeper::protocol::{self, ExitStatus, Frame, SessionSignal, SpawnRequest};
use crate::lifecycle::{Health, StatePaths, health};

pub struct ExecRequest {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: String,
}

#[derive(Debug)]
pub enum ExecError {
    /// No healthy keeper for this sandbox — `up` hasn't run (or the
    /// keeper died). Auto-up is task 5.3; this layer just reports it.
    NotRunning,
    Connect(io::Error),
    /// The keeper accepted the connection but refused to spawn (exec spec
    /// doesn't cover this directly, but `session::spawn` can still fail —
    /// e.g. `cmd` doesn't exist).
    SpawnErr(String),
    Protocol(io::Error),
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecError::NotRunning => {
                write!(f, "sandbox is not up; run `devcroft up` first")
            }
            ExecError::Connect(e) => write!(f, "connecting to keeper: {e}"),
            ExecError::SpawnErr(msg) => write!(f, "keeper refused to spawn: {msg}"),
            ExecError::Protocol(e) => write!(f, "keeper protocol error: {e}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Runs `req` inside `sandbox_name`'s running keeper, streaming this
/// process's own stdio and forwarding SIGINT/SIGTERM/SIGHUP to the
/// session's process group, and returns what devcroft's own exit code
/// should be: the child's exit code verbatim, or `128 + signal` if it
/// died by signal (exec spec's "Ctrl-C reaches the child" scenario: an
/// unhandled SIGINT is `128 + 2 = 130`, the same convention every POSIX
/// shell uses).
pub fn exec(sandbox_name: &str, req: &ExecRequest) -> Result<i32, ExecError> {
    let paths = StatePaths::new(sandbox_name).map_err(ExecError::Connect)?;
    match health(&paths).map_err(ExecError::Connect)? {
        Health::Healthy(_) => {}
        Health::Stale(_) | Health::None => return Err(ExecError::NotRunning),
    }

    let mut stream = UnixStream::connect(&paths.socket).map_err(ExecError::Connect)?;
    protocol::write_frame(
        &mut stream,
        &Frame::Spawn(SpawnRequest {
            cmd: req.cmd.clone(),
            args: req.args.clone(),
            cwd: req.cwd.clone(),
            env: BTreeMap::new(),
            pty: None,
        }),
    )
    .map_err(ExecError::Protocol)?;

    match protocol::read_frame(&mut stream).map_err(ExecError::Protocol)? {
        Frame::SpawnOk { .. } => {}
        Frame::SpawnErr { message } => return Err(ExecError::SpawnErr(message)),
        other => {
            return Err(ExecError::Protocol(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected SpawnOk, got {other:?}"),
            )));
        }
    }

    // Block SIGINT/SIGTERM/SIGHUP on this (main) thread *before* spawning
    // any other thread, so the mask — and with it, "a dedicated thread
    // handles these signals, nothing dies from the default disposition"
    // — is inherited by all of them (same technique as the keeper's own
    // shutdown handler in src/bin/devcroft.rs).
    let signal_set = block_forwarded_signals();

    let write_half = stream.try_clone().map_err(ExecError::Connect)?;
    let mut read_half = stream;

    let (tx, rx) = mpsc::channel::<Frame>();
    let writer_handle = {
        let mut w = write_half;
        thread::spawn(move || {
            for frame in rx {
                if protocol::write_frame(&mut w, &frame).is_err() {
                    break;
                }
            }
        })
    };

    spawn_stdin_pump(tx.clone());
    spawn_signal_forwarder(signal_set, tx.clone());
    drop(tx);

    let exit_code = loop {
        match protocol::read_frame(&mut read_half) {
            Ok(Frame::Stdout(bytes)) => {
                let _ = io::stdout().write_all(&bytes);
                let _ = io::stdout().flush();
            }
            Ok(Frame::Stderr(bytes)) => {
                let _ = io::stderr().write_all(&bytes);
                let _ = io::stderr().flush();
            }
            Ok(Frame::Exit(status)) => break exit_code_from(status),
            Ok(_) => {}        // no other frame is expected once a session is running
            Err(_) => break 1, // connection dropped before Exit arrived
        }
    };

    // Deliberately not joined: `spawn_signal_forwarder`'s thread blocks in
    // `sigwait` for as long as this process runs and holds its own `tx`
    // clone the whole time, so the channel never closes and
    // `writer_handle`'s `for frame in rx` loop never ends on its own —
    // joining here would deadlock forever waiting for a frame that's
    // already been sent. Nothing is lost: by the time `Exit` has arrived
    // there is nothing left worth writing, and the CLI process exits
    // immediately after this returns, which reclaims both threads anyway.
    drop(writer_handle);
    Ok(exit_code)
}

fn exit_code_from(status: ExitStatus) -> i32 {
    if let Some(code) = status.code {
        code
    } else if let Some(signal) = status.signal {
        128 + signal
    } else {
        1
    }
}

fn spawn_stdin_pump(tx: mpsc::Sender<Frame>) {
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match io::stdin().read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Frame::StdinClosed);
                    return;
                }
                Ok(n) => {
                    if tx.send(Frame::Stdin(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = tx.send(Frame::StdinClosed);
                    return;
                }
            }
        }
    });
}

fn block_forwarded_signals() -> libc::sigset_t {
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGHUP);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
    set
}

fn spawn_signal_forwarder(set: libc::sigset_t, tx: mpsc::Sender<Frame>) {
    thread::spawn(move || {
        loop {
            let mut received: libc::c_int = 0;
            let ret = unsafe { libc::sigwait(&set, &mut received) };
            if ret != 0 {
                return;
            }
            let signal = match received {
                libc::SIGINT => SessionSignal::Int,
                libc::SIGTERM => SessionSignal::Term,
                libc::SIGHUP => SessionSignal::Hup,
                _ => continue,
            };
            if tx.send(Frame::Signal(signal)).is_err() {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keeper::Keeper;

    /// `exec()` insists on a `Healthy` pidfile+socket before it ever
    /// tries the actual session protocol — the test below exercises that
    /// protocol directly against a real (test-local) `Keeper` instead of
    /// going through `exec()`, which would otherwise require faking a
    /// pidfile that points at *this* test process.
    #[test]
    fn exit_code_prefers_code_over_signal() {
        assert_eq!(
            exit_code_from(ExitStatus {
                code: Some(0),
                signal: None
            }),
            0
        );
        assert_eq!(
            exit_code_from(ExitStatus {
                code: Some(42),
                signal: None
            }),
            42
        );
    }

    #[test]
    fn exit_code_from_signal_matches_shell_convention() {
        // SIGINT = 2, and this is exactly the exec spec's "Ctrl-C reaches
        // the child" scenario: devcroft exits 130.
        assert_eq!(
            exit_code_from(ExitStatus {
                code: None,
                signal: Some(libc::SIGINT)
            }),
            130
        );
    }

    #[test]
    fn exit_code_falls_back_to_one_when_neither_is_set() {
        assert_eq!(
            exit_code_from(ExitStatus {
                code: None,
                signal: None
            }),
            1
        );
    }

    /// End-to-end against a real (in-process, unrestricted) `Keeper` —
    /// not a real keeper *process* under nono, see `tests/exec_up.rs` for
    /// that — driving the exact same wire protocol `exec()` uses so this
    /// exercises the real framing/spawn/exit path without needing
    /// `up`/nono/flox.
    #[test]
    fn protocol_roundtrip_against_a_real_keeper() {
        let sock_path =
            std::env::temp_dir().join(format!("devcroft-exec-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock_path);
        let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
        thread::spawn(move || {
            let _ = Keeper::new(listener).serve();
        });

        let mut client = UnixStream::connect(&sock_path).unwrap();
        protocol::write_frame(
            &mut client,
            &Frame::Spawn(SpawnRequest {
                cmd: "sh".to_string(),
                args: vec!["-c".to_string(), "echo hi; exit 3".to_string()],
                cwd: "/".to_string(),
                env: BTreeMap::new(),
                pty: None,
            }),
        )
        .unwrap();
        match protocol::read_frame(&mut client).unwrap() {
            Frame::SpawnOk { .. } => {}
            other => panic!("expected SpawnOk, got {other:?}"),
        }

        let mut out = Vec::new();
        let code = loop {
            match protocol::read_frame(&mut client).unwrap() {
                Frame::Stdout(bytes) => out.extend_from_slice(&bytes),
                Frame::Exit(status) => break exit_code_from(status),
                other => panic!("unexpected frame: {other:?}"),
            }
        };

        assert_eq!(String::from_utf8_lossy(&out), "hi\n");
        assert_eq!(code, 3);

        drop(client);
        let _ = std::fs::remove_file(&sock_path);
    }
}

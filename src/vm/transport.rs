//! Host-side control and keeper transport adapters (`add-vm-project-runtime`
//! task 2.3). Signals travel over `Frame::Signal` to the guest keeper rather
//! than invoking host `kill()` against guest PIDs.

use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::keeper::protocol::{
    self, ExitStatus, Frame, PtySize, QueryResult, SessionSignal, SpawnRequest,
};

use super::contract::ErrorLayer;

pub const MAX_CONTROL_LINE: usize = 4096;

/// Output collected from a bounded one-shot VM command over `keeper.sock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: ExitStatus,
}

#[derive(Debug)]
pub enum TransportError {
    ControlProtocol(String),
    SpawnRefused(String),
    Io(io::Error),
}

impl TransportError {
    pub const fn layer(&self) -> ErrorLayer {
        ErrorLayer::Keeper
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::ControlProtocol(msg) => write!(f, "VM control protocol error: {msg}"),
            TransportError::SpawnRefused(msg) => write!(f, "guest keeper refused spawn: {msg}"),
            TransportError::Io(e) => write!(f, "VM keeper transport I/O error: {e}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<io::Error> for TransportError {
    fn from(value: io::Error) -> Self {
        TransportError::Io(value)
    }
}

/// Bounded line-based client for guest PID 1 administration (`control.sock`).
#[derive(Debug, Clone)]
pub struct VmControlClient {
    pub socket_path: PathBuf,
    pub timeout: Duration,
}

impl VmControlClient {
    pub fn new(socket_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout,
        }
    }

    pub fn send_command(&self, line: &str) -> Result<String, TransportError> {
        if line.is_empty()
            || line.contains('\n')
            || line.contains('\r')
            || line.len() > MAX_CONTROL_LINE
        {
            return Err(TransportError::ControlProtocol(
                "control command must be single-line and at most 4096 bytes".into(),
            ));
        }
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream.take((MAX_CONTROL_LINE + 2) as u64));
        let mut reply = String::new();
        reader.read_line(&mut reply)?;
        if reply.len() > MAX_CONTROL_LINE + 1 {
            return Err(TransportError::ControlProtocol(
                "control reply exceeded maximum bounded line size".into(),
            ));
        }
        if !reply.ends_with('\n') {
            return Err(TransportError::ControlProtocol(
                "control socket closed without a newline-terminated reply".into(),
            ));
        }
        Ok(reply.trim_end_matches(['\r', '\n']).to_owned())
    }
}

/// Interactive streaming session over `keeper.sock` after `SpawnOk`.
pub struct VmSession {
    stream: UnixStream,
    pub session_id: u64,
}

impl VmSession {
    pub fn write_stdin(&mut self, data: &[u8]) -> Result<(), TransportError> {
        if !data.is_empty() {
            protocol::write_frame(&mut self.stream, &Frame::Stdin(data.to_vec()))?;
        }
        Ok(())
    }

    pub fn close_stdin(&mut self) -> Result<(), TransportError> {
        protocol::write_frame(&mut self.stream, &Frame::StdinClosed)?;
        Ok(())
    }

    pub fn resize(&mut self, size: PtySize) -> Result<(), TransportError> {
        protocol::write_frame(&mut self.stream, &Frame::Resize(size))?;
        Ok(())
    }

    /// Forward a guest session signal through the keeper frame protocol.
    pub fn signal(&mut self, sig: SessionSignal) -> Result<(), TransportError> {
        protocol::write_frame(&mut self.stream, &Frame::Signal(sig))?;
        Ok(())
    }

    pub fn read_frame(&mut self) -> Result<Frame, TransportError> {
        Ok(protocol::read_frame(&mut self.stream)?)
    }
}

/// Client speaking the exact devcroft keeper binary framing protocol over a
/// host-side VM Unix endpoint (`keeper.sock`).
#[derive(Debug, Clone)]
pub struct VmKeeperClient {
    pub socket_path: PathBuf,
    pub timeout: Duration,
}

impl VmKeeperClient {
    pub fn new(socket_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout,
        }
    }

    pub fn open_session(&self, req: SpawnRequest) -> Result<VmSession, TransportError> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        protocol::write_frame(&mut stream, &Frame::Spawn(req))?;
        match protocol::read_frame(&mut stream)? {
            Frame::SpawnOk { session_id } => Ok(VmSession { stream, session_id }),
            Frame::SpawnErr { message } => Err(TransportError::SpawnRefused(message)),
            other => Err(TransportError::ControlProtocol(format!(
                "expected SpawnOk or SpawnErr, got {other:?}"
            ))),
        }
    }

    pub fn run_command(
        &self,
        req: SpawnRequest,
        stdin_bytes: &[u8],
    ) -> Result<CommandOutput, TransportError> {
        let mut session = self.open_session(req)?;
        if !stdin_bytes.is_empty() {
            session.write_stdin(stdin_bytes)?;
        }
        session.close_stdin()?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = loop {
            match session.read_frame()? {
                Frame::Stdout(chunk) => stdout.extend_from_slice(&chunk),
                Frame::Stderr(chunk) => stderr.extend_from_slice(&chunk),
                Frame::Exit(status) => break status,
                other => {
                    return Err(TransportError::ControlProtocol(format!(
                        "unexpected frame during command execution: {other:?}"
                    )));
                }
            }
        };
        Ok(CommandOutput {
            stdout,
            stderr,
            exit,
        })
    }

    pub fn query(&self) -> Result<QueryResult, TransportError> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        protocol::write_frame(&mut stream, &Frame::Query)?;
        match protocol::read_frame(&mut stream)? {
            Frame::QueryResult(res) => Ok(res),
            other => Err(TransportError::ControlProtocol(format!(
                "expected QueryResult, got {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keeper::{Keeper, LocalSessionBackend};
    use std::collections::BTreeMap;
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::thread;

    fn start_test_keeper(sock: &PathBuf) {
        let listener = UnixListener::bind(sock).unwrap();
        let keeper = Keeper::new(listener, Arc::new(LocalSessionBackend));
        thread::spawn(move || {
            let _ = keeper.serve();
        });
    }

    #[test]
    fn keeper_client_handles_exec_stdin_stderr_pty_resize_and_signal() {
        let dir = std::env::temp_dir().join(format!("dc-vm-ktrans-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("keeper.sock");
        start_test_keeper(&sock);

        let client = VmKeeperClient::new(&sock, Duration::from_secs(5));
        assert_eq!(client.query().unwrap().sessions.len(), 0);

        // 1. Separated stdout, stderr, binary stdin + EOF, and nonzero exit status.
        let out = client
            .run_command(
                SpawnRequest {
                    cmd: "/bin/sh".into(),
                    args: vec!["-c".into(), "cat; echo err-line >&2; exit 7".into()],
                    cwd: "/tmp".into(),
                    env: BTreeMap::new(),
                    pty: None,
                },
                b"hello-\x00-vm",
            )
            .unwrap();
        assert_eq!(out.stdout, b"hello-\x00-vm");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err-line");
        assert_eq!(
            out.exit,
            ExitStatus {
                code: Some(7),
                signal: None
            }
        );

        // 2. PTY session with resize and Frame::Signal termination.
        let mut pty_session = client
            .open_session(SpawnRequest {
                cmd: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                cwd: "/tmp".into(),
                env: BTreeMap::new(),
                pty: Some(PtySize { rows: 24, cols: 80 }),
            })
            .unwrap();
        pty_session
            .resize(PtySize {
                rows: 40,
                cols: 120,
            })
            .unwrap();
        pty_session.signal(SessionSignal::Term).unwrap();
        let exit = loop {
            match pty_session.read_frame().unwrap() {
                Frame::Exit(st) => break st,
                Frame::Stdout(_) | Frame::Stderr(_) => continue,
                other => panic!("unexpected frame: {other:?}"),
            }
        };
        assert_eq!(exit.signal, Some(libc::SIGTERM));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn control_client_roundtrips_and_rejects_injection_or_oversized_reply() {
        let dir = std::env::temp_dir().join(format!("dc-vm-ctrl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() {
                    let mut w = conn;
                    if line.trim() == "PING" {
                        let _ = w.write_all(b"PONG\n");
                    } else {
                        let huge = vec![b'X'; MAX_CONTROL_LINE + 10];
                        let _ = w.write_all(&huge);
                        let _ = w.write_all(b"\n");
                    }
                }
            }
        });

        let client = VmControlClient::new(&sock, Duration::from_secs(2));
        assert_eq!(client.send_command("PING").unwrap(), "PONG");
        assert!(matches!(
            client.send_command("PING\nFORGED"),
            Err(TransportError::ControlProtocol(_))
        ));
        assert!(matches!(
            client.send_command("FLOOD"),
            Err(TransportError::ControlProtocol(_))
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

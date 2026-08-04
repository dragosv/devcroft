//! Wire protocol for the keeper's control socket (task 4.1): one connection
//! per session. The client sends a `Spawn` frame first; every later frame
//! flows over the same connection so `stdin`/`resize`/`signal` can be
//! multiplexed with the child's own `stdout`/`stderr` while it runs — there
//! is no separate side channel. The client is expected to close the
//! connection once it receives `Exit`.
//!
//! Framing is deliberately simple: `[1-byte tag][4-byte big-endian
//! length][payload]`. Structured frames carry a JSON payload (consistent
//! with the rest of devcroft — see the policy compiler's JSON profile);
//! `Stdin`/`Stdout`/`Stderr` carry raw bytes directly, unframed further.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};

/// Frames larger than this are rejected rather than trusted to allocate —
/// defends the keeper against a runaway or malicious peer.
const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

const TAG_SPAWN: u8 = 1;
const TAG_SPAWN_OK: u8 = 2;
const TAG_SPAWN_ERR: u8 = 3;
const TAG_STDIN: u8 = 4;
const TAG_STDIN_CLOSED: u8 = 5;
const TAG_STDOUT: u8 = 6;
const TAG_STDERR: u8 = 7;
const TAG_RESIZE: u8 = 8;
const TAG_SIGNAL: u8 = 9;
const TAG_EXIT: u8 = 10;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

/// What a session runs, requested by the client (`exec`/`shell`, tasks
/// 5.1/5.2). `env` overlays the keeper's own process environment — which
/// already carries the provider's captured activation diff (design.md
/// decision 2, "sessions inherit it for free") — rather than replacing it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpawnRequest {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    /// `None` for a piped `exec`-style session; `Some` allocates a pty
    /// sized as given for a `shell`-style session.
    pub pty: Option<PtySize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionSignal {
    Int,
    Term,
    Hup,
}

impl SessionSignal {
    pub fn as_libc(self) -> libc::c_int {
        match self {
            SessionSignal::Int => libc::SIGINT,
            SessionSignal::Term => libc::SIGTERM,
            SessionSignal::Hup => libc::SIGHUP,
        }
    }
}

/// Terminal frame for a session. Mirrors `std::process::ExitStatus`:
/// exactly one of `code`/`signal` is set on unix (never both, per
/// `WIFEXITED`/`WIFSIGNALED`), `code` alone if the wait itself failed —
/// see the `unknown` scenario in `to_exit_status`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Client -> keeper, must be the first frame on a connection.
    Spawn(SpawnRequest),
    /// Keeper -> client: the session was registered under this id.
    SpawnOk { session_id: u64 },
    /// Keeper -> client, terminal: spawning failed; the connection closes.
    SpawnErr { message: String },
    /// Client -> keeper: bytes to write to the session's stdin.
    Stdin(Vec<u8>),
    /// Client -> keeper: no more stdin; the keeper closes the child's end.
    StdinClosed,
    /// Keeper -> client: bytes read from the session's stdout (or, in pty
    /// mode, the merged stdout+stderr stream).
    Stdout(Vec<u8>),
    /// Keeper -> client: bytes read from stderr. Never sent for pty
    /// sessions — stderr is not a separate stream once merged into the pty.
    Stderr(Vec<u8>),
    /// Client -> keeper: pty resized. Ignored for piped sessions.
    Resize(PtySize),
    /// Client -> keeper: forward this signal to the session's process
    /// group.
    Signal(SessionSignal),
    /// Keeper -> client, terminal: the session's process has exited.
    Exit(ExitStatus),
}

#[derive(Serialize, Deserialize)]
struct SpawnOkPayload {
    session_id: u64,
}

#[derive(Serialize, Deserialize)]
struct SpawnErrPayload {
    message: String,
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Frame> {
    let mut tag_buf = [0u8; 1];
    r.read_exact(&mut tag_buf)?;
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds max {MAX_FRAME_LEN}"),
        ));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;

    match tag_buf[0] {
        TAG_SPAWN => Ok(Frame::Spawn(from_json(&payload)?)),
        TAG_SPAWN_OK => Ok(Frame::SpawnOk {
            session_id: from_json::<SpawnOkPayload>(&payload)?.session_id,
        }),
        TAG_SPAWN_ERR => Ok(Frame::SpawnErr {
            message: from_json::<SpawnErrPayload>(&payload)?.message,
        }),
        TAG_STDIN => Ok(Frame::Stdin(payload)),
        TAG_STDIN_CLOSED => Ok(Frame::StdinClosed),
        TAG_STDOUT => Ok(Frame::Stdout(payload)),
        TAG_STDERR => Ok(Frame::Stderr(payload)),
        TAG_RESIZE => Ok(Frame::Resize(from_json(&payload)?)),
        TAG_SIGNAL => Ok(Frame::Signal(from_json(&payload)?)),
        TAG_EXIT => Ok(Frame::Exit(from_json(&payload)?)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown frame tag {other}"),
        )),
    }
}

pub fn write_frame<W: Write>(w: &mut W, frame: &Frame) -> io::Result<()> {
    match frame {
        Frame::Spawn(req) => write_json(w, TAG_SPAWN, req),
        Frame::SpawnOk { session_id } => write_json(
            w,
            TAG_SPAWN_OK,
            &SpawnOkPayload {
                session_id: *session_id,
            },
        ),
        Frame::SpawnErr { message } => write_json(
            w,
            TAG_SPAWN_ERR,
            &SpawnErrPayload {
                message: message.clone(),
            },
        ),
        Frame::Stdin(bytes) => write_raw(w, TAG_STDIN, bytes),
        Frame::StdinClosed => write_raw(w, TAG_STDIN_CLOSED, &[]),
        Frame::Stdout(bytes) => write_raw(w, TAG_STDOUT, bytes),
        Frame::Stderr(bytes) => write_raw(w, TAG_STDERR, bytes),
        Frame::Resize(size) => write_json(w, TAG_RESIZE, size),
        Frame::Signal(sig) => write_json(w, TAG_SIGNAL, sig),
        Frame::Exit(status) => write_json(w, TAG_EXIT, status),
    }
}

fn from_json<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn write_json<W: Write, T: Serialize>(w: &mut W, tag: u8, value: &T) -> io::Result<()> {
    let payload =
        serde_json::to_vec(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_raw(w, tag, &payload)
}

fn write_raw<W: Write>(w: &mut W, tag: u8, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME_LEN as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "frame of {} bytes exceeds max {MAX_FRAME_LEN}",
                payload.len()
            ),
        ));
    }
    w.write_all(&[tag])?;
    w.write_all(&(payload.len() as u32).to_be_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn roundtrip(frame: Frame) -> Frame {
        let mut buf = Vec::new();
        write_frame(&mut buf, &frame).unwrap();
        let mut cursor = Cursor::new(buf);
        read_frame(&mut cursor).unwrap()
    }

    #[test]
    fn spawn_roundtrips() {
        let req = SpawnRequest {
            cmd: "zig".to_string(),
            args: vec!["version".to_string()],
            cwd: "/proj".to_string(),
            env: BTreeMap::from([("FOO".to_string(), "bar".to_string())]),
            pty: Some(PtySize { rows: 24, cols: 80 }),
        };
        assert_eq!(roundtrip(Frame::Spawn(req.clone())), Frame::Spawn(req));
    }

    #[test]
    fn stdin_and_stdout_roundtrip_raw_bytes() {
        assert_eq!(
            roundtrip(Frame::Stdin(vec![0, 159, 1, 255])),
            Frame::Stdin(vec![0, 159, 1, 255])
        );
        assert_eq!(
            roundtrip(Frame::Stdout(b"hello\n".to_vec())),
            Frame::Stdout(b"hello\n".to_vec())
        );
    }

    #[test]
    fn control_frames_roundtrip() {
        assert_eq!(
            roundtrip(Frame::SpawnOk { session_id: 42 }),
            Frame::SpawnOk { session_id: 42 }
        );
        assert_eq!(
            roundtrip(Frame::SpawnErr {
                message: "no such file".to_string()
            }),
            Frame::SpawnErr {
                message: "no such file".to_string()
            }
        );
        assert_eq!(roundtrip(Frame::StdinClosed), Frame::StdinClosed);
        assert_eq!(
            roundtrip(Frame::Resize(PtySize {
                rows: 40,
                cols: 120
            })),
            Frame::Resize(PtySize {
                rows: 40,
                cols: 120
            })
        );
        assert_eq!(
            roundtrip(Frame::Signal(SessionSignal::Int)),
            Frame::Signal(SessionSignal::Int)
        );
        assert_eq!(
            roundtrip(Frame::Exit(ExitStatus {
                code: Some(0),
                signal: None
            })),
            Frame::Exit(ExitStatus {
                code: Some(0),
                signal: None
            })
        );
    }

    #[test]
    fn oversized_frame_is_rejected_without_allocating() {
        let mut buf = Vec::new();
        buf.push(TAG_STDIN);
        buf.extend_from_slice(&(MAX_FRAME_LEN + 1).to_be_bytes());
        let mut cursor = Cursor::new(buf);
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn unknown_tag_is_rejected() {
        let mut buf = Vec::new();
        buf.push(0xEF);
        buf.extend_from_slice(&0u32.to_be_bytes());
        let mut cursor = Cursor::new(buf);
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncated_stream_is_unexpected_eof() {
        let mut cursor = Cursor::new(vec![TAG_STDIN, 0, 0, 0, 5, 1, 2]); // says 5 bytes, has 2
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}

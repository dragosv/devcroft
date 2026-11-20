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
//! Structured payloads use closed schemas: unknown routing/session fields
//! are refused, and the payload-free `Query`/`StdinClosed` frames require zero
//! length. Header-only errors are rejected before reading or allocating a body.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Frames larger than this are rejected rather than trusted to allocate —
/// defends the keeper against a runaway or malicious peer.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Small structured JSON control frames (`Resize`, `Signal`, `SpawnOk`,
/// `SpawnErr`, `Exit`) are bounded well below the raw stream cap.
pub const MAX_CONTROL_FRAME_LEN: u32 = 64 * 1024;

/// `Spawn` carries the command's arguments and environment, so it is bounded
/// above the platforms' `ARG_MAX` (1 MiB on macOS, 2 MiB on Linux) rather than
/// at [`MAX_CONTROL_FRAME_LEN`]: an `exec` the kernel would accept must not be
/// refused by the keeper. `QueryResult` repeats every live session's command
/// line and so keeps the stream cap.
pub const MAX_SPAWN_FRAME_LEN: u32 = 4 * 1024 * 1024;

/// Largest upfront buffer reservation before payload bytes arrive from the peer.
pub const MAX_PAYLOAD_PREALLOC: usize = 64 * 1024;

/// Monotonic deadline for completing the initial `Spawn`/`Query` frame on a new
/// connection, or for completing any subsequent frame once its tag byte arrives.
pub const FRAME_READ_DEADLINE: Duration = Duration::from_secs(1);

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
const TAG_QUERY: u8 = 11;
const TAG_QUERY_RESULT: u8 = 12;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

/// What a session runs, requested by the client (`exec`/`shell`, tasks
/// 5.1/5.2). `env` overlays the keeper's own process environment — which
/// already carries the provider's captured activation diff (design.md
/// decision 2, "sessions inherit it for free") — rather than replacing it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    /// Client -> keeper, must be the first frame on a connection (like
    /// `Spawn`, but read-only): asks for keeper uptime and the live
    /// session list, for `status`/`ps` (task 4.3). Never registers a
    /// session; the keeper answers with `QueryResult` and the connection
    /// ends there.
    Query,
    /// Keeper -> client, terminal: the answer to `Query`.
    QueryResult(QueryResult),
}

/// One live session as reported by `Query`, projected from
/// `keeper::registry::SessionInfo` (wire-friendly: a unix timestamp
/// instead of `SystemTime`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    pub id: u64,
    pub command: String,
    pub started_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QueryResult {
    pub uptime_secs: u64,
    pub sessions: Vec<SessionSummary>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnOkPayload {
    session_id: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnErrPayload {
    message: String,
}

fn validate_header(tag: u8, len: u32) -> io::Result<()> {
    if !matches!(tag, TAG_SPAWN..=TAG_QUERY_RESULT) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown frame tag {tag}"),
        ));
    }
    if matches!(tag, TAG_QUERY | TAG_STDIN_CLOSED) && len != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame tag {tag} requires an empty payload"),
        ));
    }
    let max_len = match tag {
        TAG_STDIN | TAG_STDOUT | TAG_STDERR | TAG_QUERY_RESULT => MAX_FRAME_LEN,
        TAG_SPAWN => MAX_SPAWN_FRAME_LEN,
        _ => MAX_CONTROL_FRAME_LEN,
    };
    if len > max_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds max {max_len}"),
        ));
    }
    Ok(())
}

fn decode_payload(tag: u8, payload: Vec<u8>) -> io::Result<Frame> {
    match tag {
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
        TAG_QUERY => Ok(Frame::Query),
        TAG_QUERY_RESULT => Ok(Frame::QueryResult(from_json(&payload)?)),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown frame tag {other}"),
        )),
    }
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Frame> {
    let mut tag_buf = [0u8; 1];
    r.read_exact(&mut tag_buf)?;
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    validate_header(tag_buf[0], len)?;
    let total = len as usize;
    let mut payload = Vec::with_capacity(total.min(MAX_PAYLOAD_PREALLOC));
    let mut chunk = [0u8; 8192];
    while payload.len() < total {
        let want = (total - payload.len()).min(chunk.len());
        match r.read(&mut chunk[..want]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                ));
            }
            Ok(n) => payload.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    decode_payload(tag_buf[0], payload)
}

fn read_exact_until(
    stream: &mut UnixStream,
    mut buf: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    while !buf.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "frame read deadline exceeded",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(buf) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                ));
            }
            Ok(n) => buf = &mut buf[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Reads one frame from a [`UnixStream`] with a monotonic completion deadline.
///
/// When `initial` is `true`, the deadline applies from the start of the call so an
/// idle or dribbling peer that never completes its initial `Spawn`/`Query` frame
/// is evicted. When `initial` is `false`, the stream may stay idle between frames,
/// and the monotonic deadline starts as soon as the first header byte arrives.
pub fn read_frame_timed(
    stream: &mut UnixStream,
    initial: bool,
    window: Duration,
) -> io::Result<Frame> {
    struct ResetTimeout<'a>(&'a UnixStream);
    impl Drop for ResetTimeout<'_> {
        fn drop(&mut self) {
            let _ = self.0.set_read_timeout(None);
        }
    }
    let mut tag_buf = [0u8; 1];
    let deadline = if initial {
        let deadline = Instant::now() + window;
        read_exact_until(stream, &mut tag_buf, deadline)?;
        deadline
    } else {
        stream.set_read_timeout(None)?;
        loop {
            match stream.read(&mut tag_buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed before frame tag",
                    ));
                }
                Ok(1) => break Instant::now() + window,
                Ok(_) => unreachable!(),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    };
    let mut len_buf = [0u8; 4];
    read_exact_until(stream, &mut len_buf, deadline)?;
    let len = u32::from_be_bytes(len_buf);
    validate_header(tag_buf[0], len)?;
    let total = len as usize;
    let mut payload = Vec::with_capacity(total.min(MAX_PAYLOAD_PREALLOC));
    let mut chunk = [0u8; 8192];
    while payload.len() < total {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "frame body deadline exceeded",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let want = (total - payload.len()).min(chunk.len());
        match stream.read(&mut chunk[..want]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                ));
            }
            Ok(n) => payload.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    let _reset = ResetTimeout(stream);
    decode_payload(tag_buf[0], payload)
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
        Frame::Query => write_raw(w, TAG_QUERY, &[]),
        Frame::QueryResult(result) => write_json(w, TAG_QUERY_RESULT, result),
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
        assert_eq!(roundtrip(Frame::Query), Frame::Query);
        assert_eq!(
            roundtrip(Frame::QueryResult(QueryResult {
                uptime_secs: 3600,
                sessions: vec![SessionSummary {
                    id: 1,
                    command: "sleep 100".to_string(),
                    started_unix: 1_700_000_000,
                }],
            })),
            Frame::QueryResult(QueryResult {
                uptime_secs: 3600,
                sessions: vec![SessionSummary {
                    id: 1,
                    command: "sleep 100".to_string(),
                    started_unix: 1_700_000_000,
                }],
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

    #[test]
    fn invalid_headers_are_rejected_before_requesting_a_body() {
        for (tag, length) in [
            (0xff, 4096),
            (TAG_QUERY, MAX_FRAME_LEN),
            (TAG_STDIN_CLOSED, 1),
            (TAG_STDIN, MAX_FRAME_LEN + 1),
            (TAG_SPAWN, MAX_SPAWN_FRAME_LEN + 1),
            (TAG_QUERY_RESULT, MAX_FRAME_LEN + 1),
            (TAG_RESIZE, MAX_CONTROL_FRAME_LEN + 1),
            (TAG_SIGNAL, MAX_CONTROL_FRAME_LEN + 1),
        ] {
            let mut header = vec![tag];
            header.extend_from_slice(&length.to_be_bytes());
            let mut reader = Cursor::new(header);
            assert_eq!(
                read_frame(&mut reader).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(reader.position(), 5);
        }
    }

    #[test]
    fn spawn_rejects_forged_routing_and_session_fields() {
        let normal = serde_json::json!({"cmd":"/bin/cat", "args":[], "cwd":"/workspace",
                                        "env":{}, "pty":null});
        for field in ["instance_id", "session_id", "boot_generation"] {
            let mut forged = normal.clone();
            forged[field] = serde_json::json!(42);
            let mut bytes = Vec::new();
            write_json(&mut bytes, TAG_SPAWN, &forged).unwrap();
            assert_eq!(
                read_frame(&mut Cursor::new(bytes)).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        let mut forged = normal;
        forged["pty"] = serde_json::json!({"rows":24, "cols":80, "session_id":42});
        let mut bytes = Vec::new();
        write_json(&mut bytes, TAG_SPAWN, &forged).unwrap();
        assert_eq!(
            read_frame(&mut Cursor::new(bytes)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn structured_frames_reject_unknown_authority_fields() {
        for (tag, value) in [
            (
                TAG_SPAWN_OK,
                serde_json::json!({"session_id":1,"instance_id":"other"}),
            ),
            (
                TAG_SPAWN_ERR,
                serde_json::json!({"message":"failed","instance_id":"other"}),
            ),
            (
                TAG_EXIT,
                serde_json::json!({"code":0,"signal":null,"boot_generation":1}),
            ),
            (
                TAG_RESIZE,
                serde_json::json!({"rows":24,"cols":80,"session_id":1}),
            ),
            (
                TAG_QUERY_RESULT,
                serde_json::json!({"uptime_secs":1,"sessions":[],"instance_id":"other"}),
            ),
            (
                TAG_QUERY_RESULT,
                serde_json::json!({"uptime_secs":1,"sessions":[
                {"id":1,"command":"cat","started_unix":1,"pgid":42}]}),
            ),
        ] {
            let mut bytes = Vec::new();
            write_json(&mut bytes, tag, &value).unwrap();
            assert_eq!(
                read_frame(&mut Cursor::new(bytes)).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn empty_control_frames_reject_payload_bytes() {
        for tag in [TAG_QUERY, TAG_STDIN_CLOSED] {
            let mut bytes = Vec::new();
            write_raw(&mut bytes, tag, b"forged").unwrap();
            assert_eq!(
                read_frame(&mut Cursor::new(bytes)).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn raw_stdin_bytes_do_not_change_the_next_frame_boundary() {
        let payload = vec![0xff, 0, 0, 0, 1, TAG_QUERY, 0];
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &Frame::Stdin(payload.clone())).unwrap();
        write_frame(&mut bytes, &Frame::Query).unwrap();
        let mut reader = Cursor::new(bytes);
        assert_eq!(read_frame(&mut reader).unwrap(), Frame::Stdin(payload));
        assert_eq!(read_frame(&mut reader).unwrap(), Frame::Query);
    }

    #[test]
    fn spawn_with_arguments_beyond_the_small_control_cap_is_accepted() {
        let script = "x".repeat(MAX_CONTROL_FRAME_LEN as usize + 1);
        let req = SpawnRequest {
            cmd: "sh".to_string(),
            args: vec!["-c".to_string(), script],
            cwd: "/".to_string(),
            env: BTreeMap::new(),
            pty: None,
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &Frame::Spawn(req.clone())).unwrap();
        assert_eq!(
            read_frame(&mut Cursor::new(buf)).unwrap(),
            Frame::Spawn(req)
        );
    }

    #[test]
    fn timed_read_evicts_idle_dribbling_and_mid_frame_stalls() {
        let window = Duration::from_millis(120);
        let (_writer, mut reader) = UnixStream::pair().unwrap();
        let err = read_frame_timed(&mut reader, true, window).unwrap_err();
        assert!(matches!(
            err.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));

        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        let dribbler = std::thread::spawn(move || {
            let mut header = vec![TAG_SPAWN];
            header.extend_from_slice(&32u32.to_be_bytes());
            let _ = writer.write_all(&header);
            for _ in 0..5 {
                std::thread::sleep(Duration::from_millis(35));
                if writer.write_all(b"{").is_err() {
                    break;
                }
            }
            writer
        });
        let started = Instant::now();
        let err = read_frame_timed(&mut reader, true, window).unwrap_err();
        assert!(matches!(
            err.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        assert!(started.elapsed() < Duration::from_millis(500));
        drop(dribbler.join().unwrap());

        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        let delayed = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(180));
            let _ = writer.write_all(&[TAG_STDIN, 0]);
            writer
        });
        let err = read_frame_timed(&mut reader, false, window).unwrap_err();
        assert!(matches!(
            err.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        drop(delayed.join().unwrap());
    }
}

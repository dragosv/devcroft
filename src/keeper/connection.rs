//! Drives one session end to end over one accepted connection (task 4.1):
//! reads the `Spawn` frame, spawns the child (session.rs), relays its
//! stdio as frames while servicing inbound `Resize`/`Signal`/`Stdin`
//! frames, and reaps it. A disconnected client (read error/EOF while the
//! session is still alive) gets a grace period before escalation, per the
//! exec spec's "Client killed mid-session" scenario.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use super::protocol::{self, ExitStatus, Frame, QueryResult, SessionSummary};
use super::registry::Registry;
use super::session::SessionBackend;

/// Outbound frames a session may queue ahead of its client. Each output frame
/// is at most one 8 KiB pump read, so this bounds a session's queued output at
/// 256 KiB however long its client stops reading.
const OUTPUT_QUEUE_FRAMES: usize = 32;

pub const DEFAULT_GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Handles one connection with [`DEFAULT_GRACE_PERIOD`]. See
/// [`handle_with_grace`] for the parameterized version tests use to keep
/// the disconnect-escalation path fast.
pub fn handle(
    stream: UnixStream,
    registry: Arc<Registry>,
    started: Instant,
    backend: Arc<dyn SessionBackend>,
) {
    handle_with_grace(stream, registry, started, backend, DEFAULT_GRACE_PERIOD);
}

pub fn handle_with_grace(
    stream: UnixStream,
    registry: Arc<Registry>,
    started: Instant,
    backend: Arc<dyn SessionBackend>,
    grace_period: Duration,
) {
    let mut read_half = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let write_half = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    drop(stream);

    let spawn_req =
        match protocol::read_frame_timed(&mut read_half, true, protocol::FRAME_READ_DEADLINE) {
            Ok(Frame::Spawn(req)) => req,
            Ok(Frame::Query) => {
                let sessions = registry
                    .snapshot()
                    .into_iter()
                    .map(|(id, info)| SessionSummary {
                        id,
                        command: info.command,
                        started_unix: info
                            .started
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                    })
                    .collect();
                let mut w = write_half;
                let _ = protocol::write_frame(
                    &mut w,
                    &Frame::QueryResult(QueryResult {
                        uptime_secs: started.elapsed().as_secs(),
                        sessions,
                    }),
                );
                return;
            }
            Ok(_) => {
                let mut w = write_half;
                let _ = protocol::write_frame(
                    &mut w,
                    &Frame::SpawnErr {
                        message: "expected Spawn or Query as the first frame".to_string(),
                    },
                );
                return;
            }
            Err(_) => return, // connection closed before a session ever started
        };

    let mut spawned = match backend.spawn(&spawn_req) {
        Ok(s) => s,
        Err(e) => {
            let mut w = write_half;
            let _ = protocol::write_frame(
                &mut w,
                &Frame::SpawnErr {
                    message: e.to_string(),
                },
            );
            return;
        }
    };

    let command = describe(&spawn_req);
    let session_id = registry.insert(spawned.pgid, command.clone());
    log_record(format!(
        "{} spawn session={session_id} pgid={} command={command:?}",
        log_timestamp(),
        spawned.pgid
    ));

    // Every outbound frame — SpawnOk included — flows through this one
    // channel/thread so there is a single writer for the connection's
    // lifetime; two fd duplicates racing to write would not corrupt a
    // stream socket, but funneling through one owner keeps write ordering
    // trivially obvious rather than relying on happens-before timing.
    //
    // The channel is bounded so a client that stops reading stalls the child
    // through its pipe/pty rather than growing this process: once the socket
    // and these frames are full, the pumps block, and so does the child's write.
    let (tx, rx) = mpsc::sync_channel::<Frame>(OUTPUT_QUEUE_FRAMES);
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
    if tx.send(Frame::SpawnOk { session_id }).is_err() {
        registry.remove(session_id);
        let _ = writer_handle.join();
        return;
    }

    let stdout_handle = {
        let tx = tx.clone();
        let stdout = spawned.stdout;
        thread::spawn(move || pump(stdout, tx, Frame::Stdout))
    };
    let stderr_handle = spawned.stderr.take().map(|stderr| {
        let tx = tx.clone();
        thread::spawn(move || pump(stderr, tx, Frame::Stderr))
    });

    let wait_handle = {
        let tx = tx.clone();
        let registry = Arc::clone(&registry);
        let mut child = spawned.child;
        thread::spawn(move || {
            // Both pump threads and this one send Frame values to the same
            // channel from independent threads, so nothing otherwise
            // orders Exit after the output it followed: join the pumps
            // (which finish once their fd hits EOF/EIO) before reporting
            // exit, so the client never sees Exit race ahead of buffered
            // Stdout/Stderr frames still in flight.
            let _ = stdout_handle.join();
            if let Some(h) = stderr_handle {
                let _ = h.join();
            }
            let status = to_exit_status(child.wait());
            registry.remove(session_id);
            log_record(format!(
                "{} exit session={session_id} code={:?} signal={:?}",
                log_timestamp(),
                status.code,
                status.signal
            ));
            let _ = tx.send(Frame::Exit(status));
        })
    };
    drop(tx);

    let mut stdin = spawned.stdin.take();
    let resize_handle = spawned.resize_handle.take();
    let pgid = spawned.pgid;

    loop {
        match protocol::read_frame_timed(&mut read_half, false, protocol::FRAME_READ_DEADLINE) {
            Ok(Frame::Stdin(bytes)) => {
                if let Some(w) = stdin.as_mut()
                    && w.write_all(&bytes).is_err()
                {
                    stdin = None;
                }
            }
            Ok(Frame::StdinClosed) => stdin = None,
            Ok(Frame::Resize(size)) => {
                if let Some(master) = resize_handle.as_ref() {
                    let _ = super::pty::resize(master, &size);
                }
            }
            Ok(Frame::Signal(sig)) => unsafe {
                libc::kill(-pgid, sig.as_libc());
            },
            // Each connection owns exactly one session. Unexpected requests
            // and server-only responses end it through the same cleanup path
            // as malformed input; they never select or create another session.
            Ok(_) => break,
            Err(_) => break,
        }
        if !registry.contains(session_id) {
            break;
        }
    }

    // Loop exited either because the session already finished (nothing
    // left to do) or because the client went away while it was still
    // running (grace period, then escalate).
    if registry.contains(session_id) {
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        thread::sleep(grace_period);
        if registry.contains(session_id) {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }

    let _ = wait_handle.join();
    let _ = writer_handle.join();
}

fn pump<R: Read>(mut reader: R, tx: mpsc::SyncSender<Frame>, wrap: fn(Vec<u8>) -> Frame) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            // A pty master returns EIO once its slave has closed rather
            // than a clean EOF (a well-known Linux/BSD pty quirk) — that
            // is the session ending normally, not a transport failure.
            Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
            Ok(n) => {
                if tx.send(wrap(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// `pub(crate)` so the ssh server's own channel-exit handling (task 6.3)
/// can convert a child's exit status the exact same way this control-
/// socket protocol does, rather than duplicating the signal-vs-code logic.
pub(crate) fn to_exit_status(result: std::io::Result<std::process::ExitStatus>) -> ExitStatus {
    match result {
        Ok(status) => ExitStatus {
            code: status.code(),
            signal: status.signal(),
        },
        Err(_) => ExitStatus {
            code: None,
            signal: None,
        },
    }
}

/// UTC `YYYY-MM-DDTHH:MM:SSZ`, for the spawn/exit lines `logs` (task 4.3)
/// reads back out of the keeper's own stdout/stderr (redirected to
/// `paths.log` by `up`). No time-formatting crate is vendored in this
/// workspace, and `libc::gmtime_r` is all a plain UTC stamp needs.
/// Emit one keeper log record as a single `write` call.
///
/// `eprintln!` would not do: stderr is unbuffered, so a formatted record
/// becomes one `write` per format fragment. This file has a second writer
/// — the keeper's own `stdout`, which carries hook output
/// (`hooks::run_in_keeper`) and, before `fix-service-hook-ordering`, was
/// `up` appending the same output from the host side. Either way the fd
/// is `O_APPEND`, which makes each *write* land atomically at the end but
/// says nothing about a record split across several. Interleaving then
/// lands hook output inside a keeper record, mangling both. One record,
/// one write.
fn log_record(mut line: String) {
    line.push('\n');
    let _ = std::io::stderr().write_all(line.as_bytes());
}

fn log_timestamp() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gmtime_r(&now, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

fn describe(req: &protocol::SpawnRequest) -> String {
    if req.args.is_empty() {
        req.cmd.clone()
    } else {
        format!("{} {}", req.cmd, req.args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keeper::protocol::{PtySize, SessionSignal, SpawnRequest};
    use crate::keeper::session::LocalSessionBackend;
    use std::collections::BTreeMap;
    use std::time::Instant;

    fn spawn_request(cmd: &str, args: &[&str], pty: Option<PtySize>) -> SpawnRequest {
        SpawnRequest {
            cmd: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: "/".to_string(),
            env: BTreeMap::new(),
            pty,
        }
    }

    fn expect_spawn_ok(client: &mut UnixStream) -> u64 {
        match protocol::read_frame(client).unwrap() {
            Frame::SpawnOk { session_id } => session_id,
            other => panic!("expected SpawnOk, got {other:?}"),
        }
    }

    /// Drains `Stdout`/`Stderr` frames until the terminal `Exit` frame.
    fn read_until_exit(client: &mut UnixStream) -> (Vec<u8>, ExitStatus) {
        let mut out = Vec::new();
        loop {
            match protocol::read_frame(client).unwrap() {
                Frame::Stdout(bytes) | Frame::Stderr(bytes) => out.extend_from_slice(&bytes),
                Frame::Exit(status) => return (out, status),
                other => panic!("unexpected frame while draining output: {other:?}"),
            }
        }
    }

    #[test]
    fn piped_session_echoes_stdin_and_reports_exit() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                handle(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                )
            })
        };

        protocol::write_frame(&mut client, &Frame::Spawn(spawn_request("cat", &[], None))).unwrap();
        let session_id = expect_spawn_ok(&mut client);

        protocol::write_frame(&mut client, &Frame::Stdin(b"hello\n".to_vec())).unwrap();
        protocol::write_frame(&mut client, &Frame::StdinClosed).unwrap();

        let (out, status) = read_until_exit(&mut client);
        assert_eq!(out, b"hello\n");
        assert_eq!(status.code, Some(0));
        assert_eq!(status.signal, None);

        drop(client);
        conn_thread.join().unwrap();
        assert!(!registry.contains(session_id));
    }

    /// A client that holds its connection open but stops reading (a
    /// suspended `devcroft exec`, or any peer on the socket) must stall the
    /// child, not make the keeper buffer the child's output in memory.
    #[test]
    fn a_client_that_stops_reading_stalls_the_child_instead_of_growing_the_keeper() {
        const OUTPUT: usize = 64 * 1024 * 1024;
        let marker = std::env::temp_dir().join(format!(
            "devcroft-backpressure-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        let _ = std::fs::remove_file(&marker);
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                handle(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                )
            })
        };
        let script = format!("head -c {OUTPUT} /dev/zero; : > '{}'", marker.display());
        let req = spawn_request("sh", &["-c", &script], None);
        protocol::write_frame(&mut client, &Frame::Spawn(req)).unwrap();
        expect_spawn_ok(&mut client);

        // Socket, pipe and channel buffers together are far below 64 MiB, so
        // the child can only finish here if the keeper buffers without bound.
        thread::sleep(Duration::from_secs(2));
        assert!(
            !marker.exists(),
            "the child wrote {OUTPUT} bytes while the client read none: the keeper buffered them"
        );

        // Reading again releases the child; nothing was dropped or reordered.
        let (out, status) = read_until_exit(&mut client);
        assert_eq!(out.len(), OUTPUT);
        assert_eq!(status.code, Some(0));
        assert!(marker.exists());
        let _ = std::fs::remove_file(&marker);
        drop(client);
        conn_thread.join().unwrap();
    }

    #[test]
    fn pty_session_streams_output_and_reports_exit() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                handle(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                )
            })
        };

        let req = spawn_request(
            "sh",
            &["-c", "echo hello-session"],
            Some(PtySize { rows: 24, cols: 80 }),
        );
        protocol::write_frame(&mut client, &Frame::Spawn(req)).unwrap();
        expect_spawn_ok(&mut client);

        let (out, status) = read_until_exit(&mut client);
        assert!(
            String::from_utf8_lossy(&out).contains("hello-session"),
            "expected pty output to contain the echoed text, got {out:?}"
        );
        assert_eq!(status.code, Some(0));

        drop(client);
        conn_thread.join().unwrap();
    }

    /// exec spec's "resize propagation" scenario, at the wire-protocol
    /// level: a `Resize` frame sent mid-session is applied (`pty::resize`,
    /// directly unit-tested in `pty.rs`, does the actual `TIOCSWINSZ`) and
    /// doesn't disrupt the session — output still streams and `Exit`
    /// still arrives normally.
    #[test]
    fn resize_frame_mid_session_does_not_disrupt_the_session() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                handle(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                )
            })
        };

        let req = spawn_request(
            "sh",
            &["-c", "sleep 1; echo resized-session-done"],
            Some(PtySize { rows: 24, cols: 80 }),
        );
        protocol::write_frame(&mut client, &Frame::Spawn(req)).unwrap();
        expect_spawn_ok(&mut client);

        protocol::write_frame(
            &mut client,
            &Frame::Resize(PtySize {
                rows: 40,
                cols: 100,
            }),
        )
        .unwrap();

        let (out, status) = read_until_exit(&mut client);
        assert!(
            String::from_utf8_lossy(&out).contains("resized-session-done"),
            "expected the session to keep running after a resize, got {out:?}"
        );
        assert_eq!(status.code, Some(0));

        drop(client);
        conn_thread.join().unwrap();
    }

    #[test]
    fn signal_forwarding_terminates_child_with_signal() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                handle(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                )
            })
        };

        protocol::write_frame(
            &mut client,
            &Frame::Spawn(spawn_request("sleep", &["100"], None)),
        )
        .unwrap();
        expect_spawn_ok(&mut client);

        protocol::write_frame(&mut client, &Frame::Signal(SessionSignal::Term)).unwrap();

        let (_out, status) = read_until_exit(&mut client);
        assert_eq!(status.code, None);
        assert_eq!(status.signal, Some(libc::SIGTERM));

        drop(client);
        conn_thread.join().unwrap();
    }

    #[test]
    fn client_disconnect_kills_session_after_grace_period() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                handle_with_grace(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                    Duration::from_millis(150),
                )
            })
        };

        protocol::write_frame(
            &mut client,
            &Frame::Spawn(spawn_request("sleep", &["100"], None)),
        )
        .unwrap();
        let session_id = expect_spawn_ok(&mut client);

        // Abrupt disconnect: no StdinClosed, no reading the Exit frame.
        drop(client);

        let start = Instant::now();
        conn_thread.join().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "grace-period escalation took too long: {:?}",
            start.elapsed()
        );
        assert!(!registry.contains(session_id));
    }

    #[test]
    fn query_reports_uptime_and_live_sessions_without_registering_one() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let started = Instant::now() - Duration::from_secs(5);
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || handle(server, registry, started, Arc::new(LocalSessionBackend)))
        };

        protocol::write_frame(&mut client, &Frame::Query).unwrap();
        match protocol::read_frame(&mut client).unwrap() {
            Frame::QueryResult(result) => {
                assert!(result.uptime_secs >= 5);
                assert!(result.sessions.is_empty());
            }
            other => panic!("expected QueryResult, got {other:?}"),
        }

        drop(client);
        conn_thread.join().unwrap();
        // A Query must never register a session.
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn query_after_spawn_ends_only_the_owning_connection_session() {
        let registry = Arc::new(Registry::new());
        let mut clients = Vec::new();
        let mut servers = Vec::new();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let (mut client, server) = UnixStream::pair().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let registry = Arc::clone(&registry);
            servers.push(thread::spawn(move || {
                handle_with_grace(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                    Duration::from_millis(50),
                );
            }));
            protocol::write_frame(&mut client, &Frame::Spawn(spawn_request("cat", &[], None)))
                .unwrap();
            ids.push(expect_spawn_ok(&mut client));
            clients.push(client);
        }
        // Query is permitted only as the first frame on its own connection.
        protocol::write_frame(&mut clients[0], &Frame::Query).unwrap();
        let (output, status) = read_until_exit(&mut clients[0]);
        assert!(output.is_empty());
        assert_eq!(status.signal, Some(libc::SIGTERM));
        assert!(!registry.contains(ids[0]));
        assert!(registry.contains(ids[1]));
        protocol::write_frame(&mut clients[1], &Frame::Stdin(b"still-live".to_vec())).unwrap();
        protocol::write_frame(&mut clients[1], &Frame::StdinClosed).unwrap();
        let (output, status) = read_until_exit(&mut clients[1]);
        assert_eq!(output, b"still-live");
        assert_eq!(status.code, Some(0));
        drop(clients);
        for server in servers {
            server.join().unwrap();
        }
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn mid_session_incomplete_frame_times_out_and_reaps_only_stalled_session() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::new());
        let conn_thread = {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                handle_with_grace(
                    server,
                    registry,
                    Instant::now(),
                    Arc::new(LocalSessionBackend),
                    Duration::from_millis(50),
                )
            })
        };

        protocol::write_frame(&mut client, &Frame::Spawn(spawn_request("cat", &[], None))).unwrap();
        let session_id = expect_spawn_ok(&mut client);

        // A valid Stdin header (tag 4, then a big-endian length) claiming 32
        // bytes, followed by only 4 of them, held open. A wrong tag would be
        // refused at once and never reach the deadline this test is about.
        let mut partial = vec![4u8];
        partial.extend_from_slice(&32u32.to_be_bytes());
        partial.extend_from_slice(b"part");
        client.write_all(&partial).unwrap();

        let start = Instant::now();
        let (output, status) = read_until_exit(&mut client);
        assert!(output.is_empty());
        assert_eq!(status.signal, Some(libc::SIGTERM));
        assert!(
            start.elapsed() >= protocol::FRAME_READ_DEADLINE - Duration::from_millis(100),
            "session ended before the frame deadline, so something other than it ended it: {:?}",
            start.elapsed()
        );
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "mid-session incomplete frame timeout took too long: {:?}",
            start.elapsed()
        );
        conn_thread.join().unwrap();
        assert!(!registry.contains(session_id));
    }

    #[test]
    fn connection_slot_caps_active_connections_and_recovers_after_drop() {
        use crate::keeper::{ConnectionSlot, MAX_ACTIVE_CONNECTIONS};
        use std::sync::atomic::AtomicUsize;

        let active = Arc::new(AtomicUsize::new(0));
        let mut slots = Vec::new();
        for _ in 0..MAX_ACTIVE_CONNECTIONS {
            slots.push(ConnectionSlot::try_acquire(&active).expect("slot below cap"));
        }
        assert!(
            ConnectionSlot::try_acquire(&active).is_none(),
            "connection above MAX_ACTIVE_CONNECTIONS must be refused"
        );
        slots.pop();
        assert!(
            ConnectionSlot::try_acquire(&active).is_some(),
            "dropping a slot must restore admission immediately"
        );
    }
}

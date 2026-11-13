//! Experimental fork of src/ssh/server.rs for consumed-input SSH windows.
//! The production server and its dependency remain independent of this spike.
//!
//! The embedded SSH server itself (ssh spec's "Embedded server inside the
//! boundary" requirement, task 6.1, and "SSH feature coverage for
//! editors", task 6.3): publickey auth against the single devcroft client
//! key, ephemeral host keys, unix socket only, and channels — exec,
//! pty/shell with resize and an env allowlist, exit status, the `sftp`
//! subsystem, and `-L` direct-tcpip forwarding.
//!
//! Channel handling reuses `keeper::session::spawn` — the exact primitive
//! `keeper::connection` uses for the control socket — so an `exec`/`shell`
//! session behaves identically no matter which transport it arrived over.
//! Every session starts in the keeper's own cwd, which is the project
//! root (`up` never changes directory again after `nono wrap` execs into
//! it), matching the "workspace opens at project root" scenario without
//! needing any explicit cwd negotiation over the wire — SSH doesn't carry
//! one.
//!
//! Direct-tcpip forwarding does its own policy check: none. It just tries
//! to connect, and lets the sandbox's own network restriction (already
//! applied to this whole process, same as every other syscall the keeper
//! makes) reject it if the target isn't allowed — the same posture
//! `ssh::sftp` takes with the filesystem.

use std::collections::HashMap;
use std::fs::File;
use std::future::Future;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};
use std::{io, thread};

use russh::keys::PrivateKey;
use russh::keys::ssh_key::PublicKey;
use russh::server::{self, Auth, ChannelOpenHandle, Config, Handler, Msg, Server as _, Session};
use russh::{Channel, ChannelId, ChannelOpenFailure, Pty};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::keeper::connection::{DEFAULT_GRACE_PERIOD, to_exit_status};
use crate::keeper::protocol::{ExitStatus, PtySize, SpawnRequest};
use crate::keeper::session::SessionBackend;
use crate::keeper::{pty, session};

/// The remote shell for `shell_request`. SSH carries no `$SHELL`-
/// equivalent (there's no user-account system here to look one up from
/// either). Bare `"sh"`, resolved by `PATH` inside the keeper's own
/// environment — matching `exec_request` just below, which already used
/// the bare form, and `exec.rs`'s client-side `shell()` fallback (own-
/// policy-baseline): an absolute `/bin/sh` is a host path, and is never
/// reachable once `GROUPS_EXCLUDE` declines host toolchain access,
/// regardless of what the project's provider closure supplies.
const LOGIN_SHELL: &str = "sh";

/// The shell to start a session with: the absolute path `up` resolved out
/// of this sandbox's own closure (`crate::shell`), handed over as
/// `DEVCROFT_SHELL`.
///
/// The bare-name fallback is for a keeper spawned without it — the unit
/// tests here, and a sandbox whose `up` predates this — and is the
/// behaviour this replaced: correct only when the project's closure
/// happens to supply `sh`, and otherwise resolving to a host path the
/// policy denies. Real sessions always get the absolute form, which is
/// what makes `devcroft ssh` work against a closure that declares no
/// shell of its own.
fn session_shell() -> String {
    std::env::var("DEVCROFT_SHELL").unwrap_or_else(|_| LOGIN_SHELL.to_string())
}

/// Env vars accepted from the client's `env_request` (ssh spec: "env
/// passthrough of an allowlist (TERM, LANG, LC_*)"). Anything else is
/// refused outright — a client that wants more has no way to get it,
/// which is the point of an allowlist.
fn env_allowed(name: &str) -> bool {
    name == "TERM" || name == "LANG" || name.starts_with("LC_")
}

/// Accumulated per-channel request state (`pty_request`/`env_request`)
/// that isn't acted on until `exec_request`/`shell_request` actually
/// spawns something — SSH sends these as separate, ordered requests on
/// the same channel before the one that matters.
#[derive(Default)]
struct PendingChannel {
    pty: Option<PtySize>,
    env: std::collections::BTreeMap<String, String>,
}

/// A channel with a spawned session behind it: enough to forward `data()`
/// to its stdin, `window_change_request` to its pty, and to terminate it
/// on `channel_close`.
struct ActiveSession {
    stdin: SessionStdin,
    resize_handle: Option<File>,
    _cleanup: SessionCleanup,
}

// The connection loop never waits on a child's pipe. Credits are returned
// only after the writer consumes bytes; the SSH receive window bounds pending
// input. Coalescing also bounds metadata for clients sending tiny packets.
const STDIN_BACKLOG_BYTES: usize = 8 * 1024 * 1024;
const STDIN_CHUNK_BYTES: usize = 32768;

struct InputChunk {
    bytes: Vec<u8>,
    pending: Arc<AtomicUsize>,
}

impl Drop for InputChunk {
    fn drop(&mut self) {
        self.pending.fetch_sub(self.bytes.len(), Ordering::Relaxed);
    }
}

struct StdinState {
    queue: std::collections::VecDeque<InputChunk>,
    closed: bool,
}

#[derive(Clone)]
struct SessionStdin {
    shared: Arc<(Mutex<StdinState>, Condvar)>,
    pending: Arc<AtomicUsize>,
}

impl SessionStdin {
    fn new(
        writer: Option<Box<dyn Write + Send>>,
        credit: Option<(ChannelId, server::Handle)>,
    ) -> Self {
        let input = Self {
            shared: Arc::new((
                Mutex::new(StdinState {
                    queue: Default::default(),
                    closed: writer.is_none(),
                }),
                Condvar::new(),
            )),
            pending: Arc::new(AtomicUsize::new(0)),
        };
        if let Some(mut writer) = writer {
            let shared = input.shared.clone();
            thread::spawn(move || {
                let (state, ready) = &*shared;
                loop {
                    let chunk = {
                        let mut state = state.lock().unwrap();
                        while state.queue.is_empty() && !state.closed {
                            state = ready.wait(state).unwrap();
                        }
                        state.queue.pop_front()
                    };
                    let Some(chunk) = chunk else { break };
                    if writer.write_all(&chunk.bytes).is_err() {
                        break;
                    }
                    if let Some((channel, handle)) = &credit
                        && handle.consumed(*channel, chunk.bytes.len() as u32).is_err()
                    {
                        break;
                    }
                }
                let mut state = state.lock().unwrap();
                state.closed = true;
                state.queue.clear();
            });
        }
        input
    }

    fn send(&self, mut bytes: &[u8]) -> io::Result<()> {
        let (state, ready) = &*self.shared;
        let mut state = state.lock().unwrap();
        if bytes.is_empty() || state.closed {
            return Ok(());
        }
        self.pending
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |pending| {
                pending
                    .checked_add(bytes.len())
                    .filter(|n| *n <= STDIN_BACKLOG_BYTES)
            })
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        while !bytes.is_empty() {
            if let Some(last) = state.queue.back_mut()
                && last.bytes.len() < STDIN_CHUNK_BYTES
            {
                let count = bytes.len().min(STDIN_CHUNK_BYTES - last.bytes.len());
                last.bytes.extend_from_slice(&bytes[..count]);
                bytes = &bytes[count..];
            } else {
                let count = bytes.len().min(STDIN_CHUNK_BYTES);
                state.queue.push_back(InputChunk {
                    bytes: bytes[..count].to_vec(),
                    pending: self.pending.clone(),
                });
                bytes = &bytes[count..];
            }
        }
        ready.notify_one();
        Ok(())
    }

    fn close(&self) {
        let (state, ready) = &*self.shared;
        state.lock().unwrap().closed = true;
        ready.notify_one();
    }
}

enum SessionEvent {
    OutputClosed,
    Cancel,
}

/// Channel removal and whole-connection loss both drop this guard. Signals
/// belong to the worker that owns the child, never a delayed raw PID callback:
/// it keeps the group leader unreaped until its last signal, preventing PID
/// reuse from redirecting cleanup into an unrelated session.
struct SessionCleanup(
    std::sync::mpsc::Sender<SessionEvent>,
    tokio::sync::watch::Sender<bool>,
);

impl Drop for SessionCleanup {
    fn drop(&mut self) {
        let _ = self.0.send(SessionEvent::Cancel);
        let _ = self.1.send(true);
    }
}

struct SshServer {
    authorized_key: Arc<PublicKey>,
    /// How sessions this connection spawns actually come into being —
    /// [`session::LocalSessionBackend`] for the `process` tier, or a
    /// backend's own implementation. One backend exists today; the trait
    /// is kept so a second is an addition rather than a re-architecture
    /// (`remove-gvisor-backend`, G5). See [`session::SessionBackend`].
    backend: Arc<dyn SessionBackend>,
    /// Only populated for channels that might still need the raw
    /// `Channel<Msg>` itself — currently just `sftp`'s `into_stream()`;
    /// exec/shell/direct-tcpip channels never need to be looked back up
    /// here, since all their I/O flows through `Handler::data`/`Session`
    /// instead. Per-connection: `new_client` (below) builds a fresh,
    /// empty-mapped `SshServer` for every connection rather than deriving
    /// `Clone` — `Channel<Msg>` itself isn't `Clone`, and there would be
    /// nothing to usefully share across connections in these maps anyway.
    channels: HashMap<ChannelId, Channel<Msg>>,
    pending: HashMap<ChannelId, PendingChannel>,
    active: HashMap<ChannelId, ActiveSession>,
    /// Explicit opt-in for the isolated microVM API probe. Normal keepers
    /// pass None. Only this exact guest-loopback listener may be requested;
    /// the trusted SSH client chooses its fixed host-side destination.
    remote_forward_port: Option<u16>,
    remote_forward: Option<RemoteForwardTask>,
}

struct RemoteForwardTask(tokio::task::JoinHandle<()>);

impl Drop for RemoteForwardTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl SshServer {
    fn new(authorized_key: PublicKey, backend: Arc<dyn SessionBackend>) -> Self {
        SshServer {
            authorized_key: Arc::new(authorized_key),
            backend,
            channels: HashMap::new(),
            pending: HashMap::new(),
            active: HashMap::new(),
            remote_forward_port: None,
            remote_forward: None,
        }
    }

    /// Spawns `cmd`/`args` (piped, or pty-backed if a `pty_request`
    /// already landed on this channel) with the accumulated allowlisted
    /// env, and wires up the background relay that forwards its output
    /// and exit status back over the channel. Shared by `exec_request`
    /// (a `sh -c <command>`) and `shell_request` (the login shell).
    async fn start_session(
        &mut self,
        channel_id: ChannelId,
        cmd: &str,
        args: Vec<String>,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        if self.active.contains_key(&channel_id) || !self.channels.contains_key(&channel_id) {
            session.channel_failure(channel_id)?;
            return Ok(());
        }
        session.manual_receive_window(channel_id)?;
        let pending = self.pending.remove(&channel_id).unwrap_or_default();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let req = SpawnRequest {
            cmd: cmd.to_string(),
            args,
            cwd: cwd.to_string_lossy().into_owned(),
            env: pending.env,
            pty: pending.pty,
        };

        let mut spawned = match self.backend.spawn(&req) {
            Ok(s) => s,
            Err(_) => {
                session.channel_failure(channel_id)?;
                return Ok(());
            }
        };
        // Register cleanup even if sending the acknowledgement fails after the
        // child has already been spawned.
        let acknowledged = session.channel_success(channel_id);
        // Only SFTP consumes the raw channel's receive queue. Leaving it here
        // duplicates exec input into an unread queue that stalls after 100
        // frames, even though Handler::data is consuming the same input.
        let (_, output) = self
            .channels
            .remove(&channel_id)
            .expect("validated session channel")
            .split();
        let stdin = SessionStdin::new(spawned.stdin.take(), Some((channel_id, session.handle())));
        let resize_handle = spawned.resize_handle.take();
        let cleanup = relay_session_to_channel(spawned, output, stdin.clone());

        self.active.insert(
            channel_id,
            ActiveSession {
                stdin,
                resize_handle,
                _cleanup: cleanup,
            },
        );
        acknowledged?;
        Ok(())
    }
}

/// Pumps a spawned session's stdout/stderr/exit onto its ssh channel.
/// Stdout/stderr are read synchronously on their own OS threads (mirroring
/// `keeper::connection`'s pump threads — this is the exact same "child
/// process I/O is blocking" reality, just relayed to a different
/// transport) and funneled through a `tokio` mpsc channel a spawned async
/// task drains through the channel write half, respecting its remote window. The exit-status
/// thread joins the stdout/stderr threads *before* sending `Exit`, so —
/// same guarantee `connection.rs` gives the control socket — the client
/// never sees the exit status race ahead of buffered output still in
/// flight.
fn relay_session_to_channel(
    spawned: session::SpawnedSession,
    output: russh::ChannelWriteHalf<Msg>,
    stdin: SessionStdin,
) -> SessionCleanup {
    enum Event {
        Stdout(Vec<u8>),
        Stderr(Vec<u8>),
        Exit(ExitStatus),
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(64);
    let (cleanup_tx, cleanup_rx) = std::sync::mpsc::channel();
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);

    let stdout_handle = {
        let tx = tx.clone();
        let cleanup_tx = cleanup_tx.clone();
        let mut out = spawned.stdout;
        thread::spawn(move || {
            pump(&mut out, |bytes| tx.blocking_send(Event::Stdout(bytes)));
            let _ = cleanup_tx.send(SessionEvent::OutputClosed);
        })
    };
    let output_streams = 1 + usize::from(spawned.stderr.is_some());
    let stderr_handle = spawned.stderr.map(|mut err| {
        let tx = tx.clone();
        let cleanup_tx = cleanup_tx.clone();
        thread::spawn(move || {
            pump(&mut err, |bytes| tx.blocking_send(Event::Stderr(bytes)));
            let _ = cleanup_tx.send(SessionEvent::OutputClosed);
        })
    });
    {
        let tx = tx.clone();
        let mut child = spawned.child;
        let pgid = spawned.pgid;
        thread::spawn(move || {
            let mut closed = 0;
            let status = loop {
                // Until output drains, retain the child even if it exits: its
                // PID reserves the group while descendants still hold pipes.
                let event = if closed == output_streams {
                    match child.try_wait() {
                        Ok(Some(status)) => break Ok(status),
                        Err(error) => break Err(error),
                        Ok(None) => {}
                    }
                    match cleanup_rx.recv_timeout(std::time::Duration::from_millis(20)) {
                        Ok(event) => event,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            SessionEvent::Cancel
                        }
                    }
                } else {
                    cleanup_rx.recv().unwrap_or(SessionEvent::Cancel)
                };
                match event {
                    SessionEvent::OutputClosed => closed += 1,
                    SessionEvent::Cancel => {
                        unsafe {
                            libc::kill(-pgid, libc::SIGTERM);
                        }
                        thread::sleep(DEFAULT_GRACE_PERIOD);
                        // Do not reap the leader during the grace period: the
                        // group identity must remain reserved until this call.
                        unsafe {
                            libc::kill(-pgid, libc::SIGKILL);
                        }
                        break child.wait();
                    }
                }
            };
            // Also retire an idle input writer if the client keeps a finished
            // channel alive without sending EOF.
            stdin.close();
            let _ = stdout_handle.join();
            if let Some(h) = stderr_handle {
                let _ = h.join();
            }
            let status = to_exit_status(status);
            let _ = tx.blocking_send(Event::Exit(status));
        });
    }
    drop(tx);

    let relay_cleanup = cleanup_tx.clone();
    tokio::spawn(async move {
        // Channel/connection loss also interrupts a remote-window wait. Dropping
        // rx releases pump threads even when the peer never reads stdout.
        tokio::select! {
        () = async {
        while let Some(event) = rx.recv().await {
            match event {
                Event::Stdout(bytes) => {
                    if output.data_bytes(bytes).await.is_err() {
                        let _ = relay_cleanup.send(SessionEvent::Cancel);
                        break;
                    }
                }
                // Extended data type 1 is stderr (RFC4254 5.2).
                Event::Stderr(bytes) => {
                    if output.extended_data_bytes(1, bytes).await.is_err() {
                        let _ = relay_cleanup.send(SessionEvent::Cancel);
                        break;
                    }
                }
                Event::Exit(status) => {
                    let code = status
                        .code
                        .unwrap_or_else(|| status.signal.map(|s| 128 + s).unwrap_or(1))
                        as u32;
                    let _ = output.exit_status(code).await;
                }
            }
        }
        let _ = output.eof().await;
        let _ = output.close().await;
        } => {},
        _ = stop_rx.changed() => {},
        }
    });
    SessionCleanup(cleanup_tx, stop_tx)
}

/// A blocking read loop shared by the stdout/stderr pump threads above.
/// EIO is a pty's own EOF-once-the-slave-closes quirk (same tolerance
/// `keeper::connection::pump` needs for the same reason).
fn pump<E>(reader: &mut dyn Read, mut send: impl FnMut(Vec<u8>) -> Result<(), E>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Err(e) if e.raw_os_error() == Some(libc::EIO) => return,
            Err(_) => return,
            Ok(n) => {
                if send(buf[..n].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

/// Wraps a channel stream so that a read-side EOF sends the channel's
/// exit-status *before* that EOF is allowed to propagate — the `sftp`
/// subsystem's missing completion signal (see `subsystem_request`'s doc
/// comment for why this exists). EOF is the right trigger: it arrives
/// strictly after every prior write has been flushed, because
/// `russh_sftp`'s own request loop reads the next request only once the
/// previous response is fully written.
///
/// The withholding is the load-bearing part, and it is why this is a
/// state machine rather than a `tokio::spawn`. Spawning loses a race it
/// cannot win: `russh_sftp::server::run`'s loop ends the moment it sees
/// EOF, dropping the stream and with it the channel, and a dropped russh
/// channel sends `close`. A client that already received `close` will
/// never accept a later `exit-status`. Observed directly against a real
/// `scp` before this changed — `debug2: channel 0: rcvd close` with no
/// exit-status ahead of it, and `debug1: Exit status -1` at the end, so
/// `scp` exited non-zero despite a byte-perfect transfer. Holding the EOF
/// back until the send resolves keeps the stream (and channel) alive
/// across it, which puts `exit-status` ahead of the drop's `close` by
/// construction instead of by timing.
struct NotifyOnEof<S> {
    inner: S,
    on_eof: EofState,
}

/// `Sending` owns the in-flight `exit-status` request. It is polled from
/// `poll_read` rather than awaited elsewhere so the future stays inside
/// the borrow that keeps the channel open.
enum EofState {
    Pending(ChannelId, server::Handle),
    Sending(Pin<Box<dyn Future<Output = ()> + Send>>),
    Done,
}

impl<S: AsyncRead + Unpin> AsyncRead for NotifyOnEof<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        // Resume a send begun by an earlier poll that returned Pending;
        // the real read below must not run again once EOF was observed.
        if let EofState::Sending(fut) = &mut this.on_eof {
            return match fut.as_mut().poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(()) => {
                    this.on_eof = EofState::Done;
                    Poll::Ready(Ok(()))
                }
            };
        }

        let before = buf.filled().len();
        let poll = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll
            && buf.filled().len() == before
            && let EofState::Pending(channel, handle) =
                std::mem::replace(&mut this.on_eof, EofState::Done)
        {
            // `eof` too, but never `close`: letting the stream's own drop
            // close the channel keeps a single owner of that decision, and
            // it necessarily runs after this future has resolved.
            let mut fut: Pin<Box<dyn Future<Output = ()> + Send>> = Box::pin(async move {
                let _ = handle.exit_status_request(channel, 0).await;
                let _ = handle.eof(channel).await;
            });
            return match fut.as_mut().poll(cx) {
                Poll::Pending => {
                    this.on_eof = EofState::Sending(fut);
                    Poll::Pending
                }
                Poll::Ready(()) => Poll::Ready(Ok(())),
            };
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for NotifyOnEof<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl server::Server for SshServer {
    type Handler = Self;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        SshServer {
            authorized_key: Arc::clone(&self.authorized_key),
            backend: Arc::clone(&self.backend),
            channels: HashMap::new(),
            pending: HashMap::new(),
            active: HashMap::new(),
            remote_forward_port: self.remote_forward_port,
            remote_forward: None,
        }
    }
}

impl Handler for SshServer {
    type Error = russh::Error;

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        if address != "127.0.0.1"
            || self.remote_forward_port.map(u32::from) != Some(*port)
            || self.remote_forward.is_some()
        {
            return Ok(false);
        }
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", *port as u16)).await else {
            return Ok(false);
        };
        let port = *port;
        let handle = session.handle();
        self.remote_forward = Some(RemoteForwardTask(tokio::spawn(async move {
            // The JoinSet drops/aborts every relay when this listener is
            // cancelled or its authenticated SSH connection disappears.
            let mut relays = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = relays.join_next(), if !relays.is_empty() => {},
                    accepted = listener.accept(), if relays.len() < 16 => {
                        let Ok((mut stream, peer)) = accepted else { break };
                        let handle = handle.clone();
                        relays.spawn(async move {
                            let open = handle.channel_open_forwarded_tcpip(
                                "127.0.0.1", port, peer.ip().to_string(), u32::from(peer.port()));
                            if let Ok(Ok(channel)) = tokio::time::timeout(std::time::Duration::from_secs(10), open).await {
                                let mut channel = channel.into_stream();
                                let _ = tokio::time::timeout(std::time::Duration::from_secs(120),
                                    tokio::io::copy_bidirectional(&mut stream, &mut channel)).await;
                            }
                        });
                    }
                }
            }
        })));
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        if address == "127.0.0.1" && self.remote_forward_port.map(u32::from) == Some(port) {
            return Ok(self.remote_forward.take().is_some());
        }
        Ok(false)
    }

    /// The ssh spec's whole authentication model: one recognized
    /// identity (the devcroft client keypair, `ssh::ensure_client_keypair`),
    /// checked by exact match — no username is meaningful here (the
    /// generated `ssh-config` block never sets one), since the socket's
    /// own filesystem permissions are the real access boundary and this
    /// exists only for protocol compatibility with editors/clients.
    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        // OpenSSH file comments are labels, absent from the wire key blob.
        // Comparing the whole struct rejects a correctly signed identity when
        // its authorized .pub file has a comment (ssh-keygen's usual output).
        if public_key.key_data() == self.authorized_key.key_data() {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.pending.entry(channel).or_default().pty = Some(PtySize {
            rows: row_height as u16,
            cols: col_width as u16,
        });
        session.channel_success(channel)?;
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        variable_value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if env_allowed(variable_name) {
            self.pending
                .entry(channel)
                .or_default()
                .env
                .insert(variable_name.to_string(), variable_value.to_string());
            session.channel_success(channel)?;
        } else {
            session.channel_failure(channel)?;
        }
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.start_session(channel, &session_shell(), Vec::new(), session)
            .await
    }

    /// SSH exec commands are shell command strings (what `git`, `rsync`,
    /// and legacy `scp -t`/`-f` all send), not argv arrays — run through
    /// `sh -c`, the same interpretation any real sshd gives them.
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        self.start_session(
            channel,
            &session_shell(),
            vec!["-c".to_string(), command],
            session,
        )
        .await
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(active) = self.active.get(&channel)
            && let Some(master) = active.resize_handle.as_ref()
        {
            let _ = pty::resize(
                master,
                &PtySize {
                    rows: row_height as u16,
                    cols: col_width as u16,
                },
            );
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(active) = self.active.get(&channel)
            && active.stdin.send(data).is_err()
        {
            // Fail this overloaded channel explicitly; never block the
            // connection or silently truncate a successful command's stdin.
            self.active.remove(&channel);
            session.extended_data(
                channel,
                1,
                b"devcroft: SSH stdin backlog exceeded\n".as_slice(),
            )?;
            session.exit_status_request(channel, 255)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(active) = self.active.get_mut(&channel) {
            active.stdin.close();
        }
        Ok(())
    }

    /// Dropping the channel guard requests the same bounded cleanup that
    /// dropping the whole connection does. Stdin EOF alone remains a half-close.
    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        self.pending.remove(&channel);
        self.active.remove(&channel);
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp"
            && let Some(raw_channel) = self.channels.remove(&channel)
        {
            session.channel_success(channel)?;
            // `russh_sftp::server::run` spawns its *own* background task
            // and returns immediately — it does not wait for the sftp
            // session to actually finish, and never sends an exit-status
            // or closes the channel itself either way. Sending those
            // right after `.await` "returns" races the real work: an
            // earlier version of this code did exactly that, and a real
            // `scp` (SFTP by default on OpenSSH 9+) reported "Connection
            // closed" because the channel got closed out from under an
            // in-flight transfer. `NotifyOnEof` gives a real completion
            // signal instead: it fires when the wrapped stream's read
            // side hits EOF, which is exactly when `russh_sftp`'s own
            // request loop is about to end — and by construction, after
            // every prior response was already flushed. It also withholds
            // that EOF until the exit-status has been sent, which is what
            // makes a real `scp` (unlike `sftp`, which never needed it)
            // report success; see `NotifyOnEof`'s own doc comment.
            let stream = NotifyOnEof {
                inner: raw_channel.into_stream(),
                on_eof: EofState::Pending(channel, session.handle()),
            };
            tokio::spawn(async move {
                russh_sftp::server::run(stream, super::sftp::FsHandler::default()).await;
            });
        } else {
            session.channel_failure(channel)?;
        }
        Ok(())
    }

    /// `-L` local forwarding (ssh spec): no policy check of its own here
    /// — see the module doc — just try to connect, and let the sandbox's
    /// own network restriction reject it if the target isn't allowed.
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        match tokio::net::TcpStream::connect((host_to_connect, port_to_connect as u16)).await {
            Ok(mut target) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut target).await;
                });
            }
            Err(_) => {
                reply.reject(ChannelOpenFailure::ConnectFailed).await;
            }
        }
        Ok(())
    }
}

/// Accepts connections on `listener` until it errors, running each on the
/// current tokio runtime. Never returns under normal operation — the
/// keeper is a resident process, and shutdown is the whole process
/// exiting, not this loop returning.
async fn serve(
    listener: tokio::net::UnixListener,
    host_key: PrivateKey,
    authorized_key: PublicKey,
    backend: Arc<dyn SessionBackend>,
    remote_forward_port: Option<u16>,
) -> io::Result<()> {
    let config = Arc::new(Config {
        keys: vec![host_key],
        ..Default::default()
    });
    let mut server = SshServer::new(authorized_key, backend);
    server.remote_forward_port = remote_forward_port;

    loop {
        let (stream, _addr) = listener.accept().await?;
        let handler = server.new_client(None);
        let config = Arc::clone(&config);
        tokio::spawn(async move {
            // A connection setup failure here (garbage on the wire, a kex
            // failure, etc.) is one bad connection attempt, not a reason
            // to take the whole server down — nothing to do but drop it.
            if let Ok(session) = server::run_stream(config, stream, handler).await {
                let _ = session.await;
            }
        });
    }
}

/// Spawns the SSH server on its own OS thread with a dedicated tokio
/// runtime, and returns immediately — the rest of the keeper (task 4.1)
/// is synchronous/thread-per-connection, so this is deliberately kept
/// separate rather than pulling the whole keeper onto an async runtime.
/// `std_listener` must already be in the state expected of an inherited,
/// pre-bound fd (see `up.rs`): non-blocking mode is set here since a
/// plain inherited `std::os::unix::net::UnixListener` is blocking by
/// default and tokio requires non-blocking.
/// The SSH server's runtime workers: fixed, whatever the host's CPU count.
pub const SSH_WORKER_THREADS: usize = 2;

pub fn spawn(
    std_listener: std::os::unix::net::UnixListener,
    host_key: PrivateKey,
    authorized_key: PublicKey,
    backend: Arc<dyn SessionBackend>,
    remote_forward_port: Option<u16>,
) {
    if let Err(e) = std_listener.set_nonblocking(true) {
        eprintln!("devcroft: ssh: set_nonblocking on the ssh socket: {e}");
        return;
    }
    std::thread::spawn(move || {
        // A fixed pool, not one worker per CPU (`Runtime::new`'s default).
        // Every thread counts against the agent's `pids.max`, which the
        // keeper shares with the workload, so the default made the control
        // plane's footprint grow with the host: on a 22-CPU host an agent
        // with `pids.max` 32 could not start a shell (adversarial review
        // §4.2). Two workers serve an editor's connections; measured, an
        // idle agent went from 16 pids to 6 on a 12-CPU host.
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(SSH_WORKER_THREADS)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("devcroft: ssh: starting tokio runtime: {e}");
                return;
            }
        };
        rt.block_on(async move {
            let listener = match tokio::net::UnixListener::from_std(std_listener) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("devcroft: ssh: adopting the ssh socket into tokio: {e}");
                    return;
                }
            };
            if let Err(e) = serve(
                listener,
                host_key,
                authorized_key,
                backend,
                remote_forward_port,
            )
            .await
            {
                eprintln!("devcroft: ssh: server exited: {e}");
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::client;
    use russh::keys::Algorithm;
    use russh::keys::key::PrivateKeyWithHashAlg;

    /// Accepts any server host key (this crate's own client — `proxy`,
    /// task 6.2 — never verifies one either: `ssh-config`'s
    /// `StrictHostKeyChecking no` already tells a real ssh client not to,
    /// and the socket's filesystem permissions are the actual boundary).
    struct AcceptAnyServerKey;

    impl client::Handler for AcceptAnyServerKey {
        type Error = russh::Error;

        async fn check_server_key(&mut self, _: &PublicKey) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    /// Runs the server handler directly over one half of an in-memory
    /// duplex pair — the same `run_stream` entry point `serve` uses per
    /// connection, just without a real `UnixListener` in front of it,
    /// which is exactly the trade `tests/exec.rs`'s `Keeper` tests make
    /// too (protocol-level coverage without a real process boundary; a
    /// real `up`-driven connection is `tests/ssh_up.rs`).
    async fn authenticated_connection(
        client_key: PrivateKey,
        authorized_key: PublicKey,
    ) -> (client::Handle<AcceptAnyServerKey>, bool) {
        let (client, authenticated, _) =
            authenticated_connection_with_transport(client_key, authorized_key).await;
        (client, authenticated)
    }

    async fn authenticated_connection_with_transport(
        client_key: PrivateKey,
        authorized_key: PublicKey,
    ) -> (
        client::Handle<AcceptAnyServerKey>,
        bool,
        std::os::unix::net::UnixStream,
    ) {
        let (client_stream, server_stream) = tokio::net::UnixStream::pair().unwrap();
        let client_stream = client_stream.into_std().unwrap();
        let transport = client_stream.try_clone().unwrap();
        let client_stream = tokio::net::UnixStream::from_std(client_stream).unwrap();

        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let server_config = Arc::new(Config {
            keys: vec![host_key],
            ..Default::default()
        });
        let server_handler = SshServer::new(authorized_key, Arc::new(session::LocalSessionBackend));
        tokio::spawn(async move {
            if let Ok(session) =
                server::run_stream(server_config, server_stream, server_handler).await
            {
                let _ = session.await;
            }
        });

        let client_config = Arc::new(client::Config::default());
        let mut handle = client::connect_stream(client_config, client_stream, AcceptAnyServerKey)
            .await
            .unwrap();
        let result = handle
            .authenticate_publickey(
                "devcroft",
                PrivateKeyWithHashAlg::new(Arc::new(client_key), None),
            )
            .await
            .unwrap();
        (handle, result.success(), transport)
    }

    async fn authenticate(client_key: PrivateKey, authorized_key: PublicKey) -> bool {
        authenticated_connection(client_key, authorized_key).await.1
    }

    #[test]
    fn stdin_backlog_is_bounded_and_eof_drains_accepted_input() {
        struct HeldWriter {
            release: Option<mpsc::Receiver<()>>,
            entered: mpsc::Sender<()>,
            finished: mpsc::Sender<Vec<u8>>,
            bytes: Vec<u8>,
        }
        impl Write for HeldWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(release) = self.release.take() {
                    self.entered.send(()).unwrap();
                    release
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl Drop for HeldWriter {
            fn drop(&mut self) {
                let _ = self.finished.send(std::mem::take(&mut self.bytes));
            }
        }
        for byte_limit in [false, true] {
            let (release, released) = mpsc::channel();
            let (entered, writing) = mpsc::channel();
            let (finished, done) = mpsc::channel();
            let input = SessionStdin::new(
                Some(Box::new(HeldWriter {
                    release: Some(released),
                    entered,
                    finished,
                    bytes: Vec::new(),
                })),
                None,
            );
            let first = vec![7; if byte_limit { STDIN_BACKLOG_BYTES } else { 1 }];
            input.send(&first).unwrap();
            writing
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            if !byte_limit {
                for _ in 0..4096 {
                    input.send(&[7]).unwrap();
                }
            }
            assert_eq!(
                input
                    .send(&vec![9; if byte_limit { 1 } else { STDIN_BACKLOG_BYTES }])
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::WouldBlock
            );
            // A second owner models the child-wait worker: EOF must close the
            // actual sender despite that owner, and drain the accepted bytes.
            let owner = input.clone();
            input.close();
            input.send(&[10]).unwrap();
            release.send(()).unwrap();
            let bytes = done
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            let expected = if byte_limit {
                STDIN_BACKLOG_BYTES
            } else {
                4097
            };
            assert_eq!(bytes, vec![7; expected]);
            assert_eq!(owner.pending.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nonreading_stdin_does_not_block_other_channels() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let (client, authorized) =
            authenticated_connection(key.clone(), key.public_key().clone()).await;
        assert!(authorized);
        let mut channel = client.channel_open_session().await.unwrap();
        channel
            .exec(true, "trap '' TERM; printf '%s\\n' \"$$\"; exec sleep 30")
            .await
            .unwrap();
        let pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut output = Vec::new();
            loop {
                if let russh::ChannelMsg::Data { data } = channel.wait().await.unwrap() {
                    output.extend_from_slice(&data);
                    if output.contains(&b'\n') {
                        break String::from_utf8(output)
                            .unwrap()
                            .trim()
                            .parse::<libc::pid_t>()
                            .unwrap();
                    }
                }
            }
        })
        .await
        .unwrap();
        assert!(pid > 1);
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (_, writer) = channel.split();
            let send = writer.data_bytes(vec![1; 12 * 1024 * 1024]);
            let other = async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let mut next = client.channel_open_session().await.unwrap();
                next.exec(true, "printf responsive").await.unwrap();
                let mut output = Vec::new();
                loop {
                    match next.wait().await.unwrap() {
                        russh::ChannelMsg::Data { data } => output.extend_from_slice(&data),
                        russh::ChannelMsg::ExitStatus { exit_status } => {
                            assert_eq!(exit_status, 0);
                            break;
                        }
                        _ => {}
                    }
                }
                assert_eq!(output, b"responsive");
                writer.close().await.unwrap();
            };
            tokio::select! {
                result = send => panic!("nonreading sink accepted twelve MiB: {result:?}"),
                () = other => {},
            }
        })
        .await
        .unwrap();
        let gone = tokio::time::timeout(std::time::Duration::from_secs(7), async {
            while unsafe { libc::kill(pid, 0) } != -1
                || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        if gone.is_err() {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        assert!(
            gone.is_ok(),
            "blocked child's process group survived channel close"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exhausted_stdout_window_does_not_block_another_channel_or_cleanup() {
        struct FreezeWindow;
        impl client::Handler for FreezeWindow {
            type Error = russh::Error;
            async fn check_server_key(&mut self, _: &PublicKey) -> Result<bool, Self::Error> {
                Ok(true)
            }
            fn adjust_window(&mut self, _: ChannelId, _: u32) -> u32 {
                1
            }
        }
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let (client_stream, server_stream) = tokio::net::UnixStream::pair().unwrap();
        let config = Arc::new(Config {
            keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
            ..Default::default()
        });
        let handler = SshServer::new(
            key.public_key().clone(),
            Arc::new(session::LocalSessionBackend),
        );
        tokio::spawn(async move {
            let _ = server::run_stream(config, server_stream, handler)
                .await
                .unwrap()
                .await;
        });
        let mut client = client::connect_stream(
            Arc::new(client::Config {
                window_size: 65536,
                ..Default::default()
            }),
            client_stream,
            FreezeWindow,
        )
        .await
        .unwrap();
        assert!(
            client
                .authenticate_publickey("devcroft", PrivateKeyWithHashAlg::new(Arc::new(key), None))
                .await
                .unwrap()
                .success()
        );
        let mut channel = client.channel_open_session().await.unwrap();
        channel
            .exec(
                true,
                "trap '' TERM; printf '%s\\n' \"$$\"; exec head -c 8388608 /dev/zero",
            )
            .await
            .unwrap();
        let mut first = Vec::new();
        let pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let russh::ChannelMsg::Data { data } = channel.wait().await.unwrap() {
                    first.extend_from_slice(&data);
                    if let Some(end) = first.iter().position(|b| *b == b'\n') {
                        break std::str::from_utf8(&first[..end])
                            .unwrap()
                            .parse::<libc::pid_t>()
                            .unwrap();
                    }
                }
            }
        })
        .await
        .unwrap();
        assert!(pid > 1);
        let received = Arc::new(AtomicUsize::new(first.len()));
        let observed = received.clone();
        let (mut reader, writer) = channel.split();
        let drain = tokio::spawn(async move {
            while let Some(message) = reader.wait().await {
                match message {
                    russh::ChannelMsg::Data { data } => {
                        received.fetch_add(data.len(), Ordering::Relaxed);
                    }
                    russh::ChannelMsg::Close => break,
                    _ => {}
                }
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while observed.load(Ordering::Relaxed) < 65536 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            assert!(
                observed.load(Ordering::Relaxed) < 1024 * 1024,
                "negative window control did not stop output"
            );
            assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
            let mut other = client.channel_open_session().await.unwrap();
            other.exec(true, "printf independent").await.unwrap();
            let mut output = Vec::new();
            loop {
                match other.wait().await.unwrap() {
                    russh::ChannelMsg::Data { data } => output.extend_from_slice(&data),
                    russh::ChannelMsg::ExitStatus { exit_status } => {
                        assert_eq!(exit_status, 0);
                        break;
                    }
                    _ => {}
                }
            }
            assert_eq!(output, b"independent");
            writer.close().await.unwrap();
        })
        .await
        .unwrap();
        let gone = tokio::time::timeout(std::time::Duration::from_secs(7), async {
            while unsafe { libc::kill(pid, 0) } != -1
                || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        if gone.is_err() {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        assert!(gone.is_ok(), "window-blocked child survived channel close");
        tokio::time::timeout(std::time::Duration::from_secs(5), drain)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn normal_keeper_rejects_remote_forwarding() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let (client, authorized) =
            authenticated_connection(key.clone(), key.public_key().clone()).await;
        assert!(authorized);
        assert!(client.tcpip_forward("127.0.0.1", 8082).await.is_err());
    }

    #[tokio::test]
    async fn transport_loss_or_channel_close_reaps_piped_and_pty_sessions() {
        for (pty, close_channel) in [(false, false), (true, false), (false, true), (true, true)] {
            let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
            let (client, authorized, transport) =
                authenticated_connection_with_transport(key.clone(), key.public_key().clone())
                    .await;
            assert!(authorized);
            let mut channel = client.channel_open_session().await.unwrap();
            if pty {
                channel
                    .request_pty(false, "xterm", 80, 24, 0, 0, &[])
                    .await
                    .unwrap();
            }
            // Ignore TERM so this proves escalation as well as connection-drop
            // notification. exec preserves the PID printed by the shell.
            channel
                .exec(
                    true,
                    "trap '' TERM; printf 'SESSION_PID=%s\\n' \"$$\"; exec sleep 30",
                )
                .await
                .unwrap();
            let pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut output = Vec::new();
                loop {
                    match channel.wait().await.unwrap() {
                        russh::ChannelMsg::Data { data } => output.extend_from_slice(&data),
                        russh::ChannelMsg::ExitStatus { exit_status } => {
                            panic!("early exit {exit_status}")
                        }
                        _ => {}
                    }
                    if output.contains(&b'\n') {
                        break String::from_utf8(output)
                            .unwrap()
                            .trim()
                            .strip_prefix("SESSION_PID=")
                            .unwrap()
                            .parse::<libc::pid_t>()
                            .unwrap();
                    }
                }
            })
            .await
            .unwrap();
            assert!(pid > 1);
            assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "positive control missing");
            if close_channel {
                channel.close().await.unwrap();
            } else {
                // No SSH disconnect or channel-close packet: destroy the
                // transport while leaving the client handle/channel alive.
                transport.shutdown(std::net::Shutdown::Both).unwrap();
            }
            let gone = tokio::time::timeout(std::time::Duration::from_secs(7), async {
                loop {
                    if unsafe { libc::kill(pid, 0) } == -1
                        && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await;
            if gone.is_err() {
                // This is the still-live group identified by the positive
                // control, well before its 30-second natural termination.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
            assert!(
                gone.is_ok(),
                "session survived cleanup (pty={pty}, close_channel={close_channel})"
            );
            if close_channel {
                // Channel cleanup must leave this authenticated connection and
                // independently opened sessions usable.
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    let mut next = client.channel_open_session().await.unwrap();
                    next.exec(true, "printf still-connected").await.unwrap();
                    let mut output = Vec::new();
                    loop {
                        match next.wait().await.unwrap() {
                            russh::ChannelMsg::Data { data } => {
                                output.extend_from_slice(&data);
                            }
                            russh::ChannelMsg::ExitStatus { exit_status } => {
                                assert_eq!(exit_status, 0);
                                break;
                            }
                            _ => {}
                        }
                    }
                    assert_eq!(output, b"still-connected");
                })
                .await
                .unwrap();
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_duplex_bursts_and_window_bounded_streams_do_not_block_the_connection() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let (client, authorized) =
            authenticated_connection(key.clone(), key.public_key().clone()).await;
        assert!(authorized);
        // Both directions must progress without application-level echo credits.
        // The large case crosses the SSH window and socket capacity repeatedly.
        for size in [131072, 16 * 1024 * 1024] {
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                let mut channel = client.channel_open_session().await.unwrap();
                channel.exec(true, "cat; exit 29").await.unwrap();
                loop {
                    match channel.wait().await.unwrap() {
                        russh::ChannelMsg::Success => break,
                        russh::ChannelMsg::Failure => panic!("exec request refused"),
                        _ => {}
                    }
                }
                let input = (0..size)
                    .map(|index| (index % 251) as u8)
                    .collect::<Vec<_>>();
                let (mut reader, writer) = channel.split();
                let send = async {
                    writer.data_bytes(input.clone()).await.unwrap();
                    writer.eof().await.unwrap();
                };
                let receive = async {
                    let mut output = Vec::new();
                    let mut status = None;
                    while let Some(message) = reader.wait().await {
                        match message {
                            russh::ChannelMsg::Data { data } => {
                                output.extend_from_slice(&data);
                            }
                            russh::ChannelMsg::ExitStatus { exit_status } => {
                                assert_eq!(output, input, "exit status preceded buffered output");
                                status = Some(exit_status);
                            }
                            russh::ChannelMsg::Close => break,
                            _ => {}
                        }
                    }
                    assert_eq!(output, input);
                    assert_eq!(status, Some(29));
                };
                tokio::join!(send, receive);
            })
            .await
            .expect("full-duplex transfer timed out");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stdin_eof_drains_buffered_output_before_exit_status() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let (client, authorized) =
            authenticated_connection(key.clone(), key.public_key().clone()).await;
        assert!(authorized);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut channel = client.channel_open_session().await.unwrap();
            // Accumulate stdin until EOF, then emit a large buffered response.
            // This isolates half-close/exit ordering from stdin/stdout duplex
            // backpressure in a native shell's cat implementation.
            channel
                .exec(true, "output=$(cat); printf %s \"$output\"; exit 29")
                .await
                .unwrap();
            loop {
                match channel.wait().await.unwrap() {
                    russh::ChannelMsg::Success => break,
                    russh::ChannelMsg::Failure => panic!("exec request refused"),
                    _ => {}
                }
            }
            let input = (0..131072)
                .map(|index| b'a' + (index % 26) as u8)
                .collect::<Vec<_>>();
            channel.data_bytes(input.clone()).await.unwrap();
            channel.eof().await.unwrap();
            let mut output = Vec::new();
            let mut status = None;
            while let Some(message) = channel.wait().await {
                match message {
                    russh::ChannelMsg::Data { data } => output.extend_from_slice(&data),
                    russh::ChannelMsg::ExitStatus { exit_status } => {
                        assert_eq!(output, input, "exit status preceded buffered output");
                        status = Some(exit_status);
                    }
                    russh::ChannelMsg::Close => break,
                    _ => {}
                }
            }
            assert_eq!(output, input);
            assert_eq!(status, Some(29));
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "native OpenSSH/Python regression; run explicitly where both tools are installed"]
    async fn openssh_streams_sixteen_mib_bidirectionally() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt;

        let path = std::env::temp_dir().join(format!(
            "dc-duplex-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let client_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let key_path = path.join("client");
        std::fs::write(
            &key_path,
            client_key
                .to_openssh(russh::keys::ssh_key::LineEnding::LF)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let known_hosts = path.join("known_hosts");
        std::fs::write(
            &known_hosts,
            format!(
                "duplex-guest {}\n",
                host_key.public_key().to_openssh().unwrap()
            ),
        )
        .unwrap();
        let socket = path.join("ssh.sock");
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let proxy = manifest
            .ancestors()
            .map(|p| p.join("experiments/microvm/ssh_transport.py"))
            .find(|p| p.is_file())
            .expect("experimental SSH proxy fixture missing");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(serve(
            listener,
            host_key,
            client_key.public_key().clone(),
            Arc::new(session::LocalSessionBackend),
            None,
        ));
        let result = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            let mut child = tokio::process::Command::new("ssh")
                .args([
                    "-F",
                    "/dev/null",
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "IdentitiesOnly=yes",
                    "-o",
                    "IdentityAgent=none",
                    "-o",
                    "ForwardAgent=no",
                    "-o",
                    "StrictHostKeyChecking=yes",
                    "-o",
                    "GlobalKnownHostsFile=/dev/null",
                    "-o",
                    "HostKeyAlias=duplex-guest",
                    "-o",
                    "ConnectTimeout=5",
                    "-o",
                    "ConnectionAttempts=1",
                ])
                .arg("-o")
                .arg(format!("UserKnownHostsFile={}", known_hosts.display()))
                .arg("-o")
                .arg(format!(
                    "ProxyCommand=python3 {} {}",
                    proxy.display(),
                    socket.display()
                ))
                .arg("-i")
                .arg(&key_path)
                .args(["devcroft@guest", "cat; exit 29"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let input = (0..16 * 1024 * 1024)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            let mut stdin = child.stdin.take().unwrap();
            let send = async {
                stdin.write_all(&input).await.unwrap();
                stdin.shutdown().await.unwrap();
                drop(stdin);
            };
            let (_, received) = tokio::join!(send, child.wait_with_output());
            let received = received.unwrap();
            assert_eq!(
                received.status.code(),
                Some(29),
                "{}",
                String::from_utf8_lossy(&received.stderr)
            );
            assert_eq!(received.stdout, input);
        })
        .await;
        server.abort();
        let _ = server.await;
        std::fs::remove_dir_all(path).unwrap();
        result.expect("native OpenSSH full-duplex transfer timed out");
    }

    #[tokio::test]
    async fn the_authorized_key_is_accepted() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        assert!(authenticate(key.clone(), key.public_key().clone()).await);
    }

    #[tokio::test]
    async fn an_authorized_key_file_comment_is_not_part_of_the_wire_identity() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let mut authorized = key.public_key().clone();
        authorized.set_comment("ssh-keygen-label-not-sent-over-the-wire");
        assert!(authenticate(key, authorized).await);
    }

    #[tokio::test]
    async fn any_other_key_is_rejected() {
        let authorized = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let other = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        assert!(!authenticate(other, authorized.public_key().clone()).await);
    }
}

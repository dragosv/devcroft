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
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{io, thread};

use russh::keys::PrivateKey;
use russh::keys::ssh_key::PublicKey;
use russh::server::{self, Auth, ChannelOpenHandle, Config, Handler, Msg, Server as _, Session};
use russh::{Channel, ChannelId, ChannelOpenFailure, Pty};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::keeper::connection::to_exit_status;
use crate::keeper::protocol::{ExitStatus, PtySize, SpawnRequest};
use crate::keeper::session::SessionBackend;
use crate::keeper::{pty, session};

/// The remote shell for `shell_request`/pty-less `exec_request` with no
/// pty. SSH carries no `$SHELL`-equivalent (there's no user-account
/// system here to look one up from either) — same fallback `exec.rs`'s
/// client-side `shell()` uses for the identical reason.
const LOGIN_SHELL: &str = "/bin/sh";

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
    stdin: Option<Box<dyn Write + Send>>,
    resize_handle: Option<File>,
    pgid: libc::pid_t,
}

struct SshServer {
    authorized_key: Arc<PublicKey>,
    /// How sessions this connection spawns actually come into being —
    /// [`session::LocalSessionBackend`] for the `process` tier, or a
    /// hardened backend's own implementation (e.g. `runsc exec`) for the
    /// `hardened` tier. See [`session::SessionBackend`].
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
}

impl SshServer {
    fn new(authorized_key: PublicKey, backend: Arc<dyn SessionBackend>) -> Self {
        SshServer {
            authorized_key: Arc::new(authorized_key),
            backend,
            channels: HashMap::new(),
            pending: HashMap::new(),
            active: HashMap::new(),
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
        session.channel_success(channel_id)?;

        self.active.insert(
            channel_id,
            ActiveSession {
                stdin: spawned.stdin.take(),
                resize_handle: spawned.resize_handle.take(),
                pgid: spawned.pgid,
            },
        );

        relay_session_to_channel(channel_id, spawned, session.handle());
        Ok(())
    }
}

/// Pumps a spawned session's stdout/stderr/exit onto its ssh channel.
/// Stdout/stderr are read synchronously on their own OS threads (mirroring
/// `keeper::connection`'s pump threads — this is the exact same "child
/// process I/O is blocking" reality, just relayed to a different
/// transport) and funneled through a `tokio` mpsc channel a spawned async
/// task drains into `Handle::data`/`extended_data` calls. The exit-status
/// thread joins the stdout/stderr threads *before* sending `Exit`, so —
/// same guarantee `connection.rs` gives the control socket — the client
/// never sees the exit status race ahead of buffered output still in
/// flight.
fn relay_session_to_channel(
    channel_id: ChannelId,
    spawned: session::SpawnedSession,
    handle: server::Handle,
) {
    enum Event {
        Stdout(Vec<u8>),
        Stderr(Vec<u8>),
        Exit(ExitStatus),
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(64);

    let stdout_handle = {
        let tx = tx.clone();
        let mut out = spawned.stdout;
        thread::spawn(move || pump(&mut out, |bytes| tx.blocking_send(Event::Stdout(bytes))))
    };
    let stderr_handle = spawned.stderr.map(|mut err| {
        let tx = tx.clone();
        thread::spawn(move || pump(&mut err, |bytes| tx.blocking_send(Event::Stderr(bytes))))
    });
    {
        let tx = tx.clone();
        let mut child = spawned.child;
        thread::spawn(move || {
            let _ = stdout_handle.join();
            if let Some(h) = stderr_handle {
                let _ = h.join();
            }
            let status = to_exit_status(child.wait());
            let _ = tx.blocking_send(Event::Exit(status));
        });
    }
    drop(tx);

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                Event::Stdout(bytes) => {
                    let _ = handle.data(channel_id, bytes).await;
                }
                // Extended data type 1 is stderr (RFC4254 5.2).
                Event::Stderr(bytes) => {
                    let _ = handle.extended_data(channel_id, 1, bytes).await;
                }
                Event::Exit(status) => {
                    let code = status
                        .code
                        .unwrap_or_else(|| status.signal.map(|s| 128 + s).unwrap_or(1))
                        as u32;
                    let _ = handle.exit_status_request(channel_id, code).await;
                }
            }
        }
        let _ = handle.eof(channel_id).await;
        let _ = handle.close(channel_id).await;
    });
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
        }
    }
}

impl Handler for SshServer {
    type Error = russh::Error;

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
        if public_key == self.authorized_key.as_ref() {
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
        self.start_session(channel, LOGIN_SHELL, Vec::new(), session)
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
        self.start_session(channel, "sh", vec!["-c".to_string(), command], session)
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
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(active) = self.active.get_mut(&channel)
            && let Some(stdin) = active.stdin.as_mut()
            && stdin.write_all(data).is_err()
        {
            active.stdin = None;
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(active) = self.active.get_mut(&channel) {
            active.stdin = None;
        }
        Ok(())
    }

    /// Terminates the session's process group unconditionally — a no-op
    /// (`ESRCH`, silently ignored) if it had already exited on its own,
    /// and orphan cleanup if the client went away mid-session. Simpler
    /// than the control socket's grace-period escalation (`connection.rs`)
    /// and sufficient here: `channel_close` only fires once the client is
    /// truly done with this channel, unlike a raw disconnect the control
    /// socket has to distinguish from "still attached".
    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        self.pending.remove(&channel);
        if let Some(active) = self.active.remove(&channel) {
            unsafe {
                libc::kill(-active.pgid, libc::SIGTERM);
            }
        }
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
) -> io::Result<()> {
    let config = Arc::new(Config {
        keys: vec![host_key],
        ..Default::default()
    });
    let mut server = SshServer::new(authorized_key, backend);

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
pub fn spawn(
    std_listener: std::os::unix::net::UnixListener,
    host_key: PrivateKey,
    authorized_key: PublicKey,
    backend: Arc<dyn SessionBackend>,
) {
    if let Err(e) = std_listener.set_nonblocking(true) {
        eprintln!("devcroft: ssh: set_nonblocking on the ssh socket: {e}");
        return;
    }
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
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
            if let Err(e) = serve(listener, host_key, authorized_key, backend).await {
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
    async fn authenticate(client_key: PrivateKey, authorized_key: PublicKey) -> bool {
        let (client_stream, server_stream) = tokio::net::UnixStream::pair().unwrap();

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
        result.success()
    }

    #[tokio::test]
    async fn the_authorized_key_is_accepted() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        assert!(authenticate(key.clone(), key.public_key().clone()).await);
    }

    #[tokio::test]
    async fn any_other_key_is_rejected() {
        let authorized = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let other = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        assert!(!authenticate(other, authorized.public_key().clone()).await);
    }
}

//! The embedded SSH server itself (ssh spec's "Embedded server inside the
//! boundary" requirement, task 6.1): publickey auth against the single
//! devcroft client key, ephemeral host keys, unix socket only. Channel
//! handling (exec/pty/sftp/forwarding — task 6.3) isn't implemented yet;
//! every [`Handler`] method but `auth_publickey` uses russh's own default
//! (reject), so an authenticated client can connect but can't yet open a
//! channel — the point of this task is the listener and the handshake.

use std::io;
use std::sync::Arc;

use russh::keys::PrivateKey;
use russh::keys::ssh_key::PublicKey;
use russh::server::{self, Auth, Config, Handler, Server as _};

#[derive(Clone)]
struct SshServer {
    authorized_key: Arc<PublicKey>,
}

impl server::Server for SshServer {
    type Handler = Self;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        self.clone()
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
}

/// Accepts connections on `listener` until it errors, running each on the
/// current tokio runtime. Never returns under normal operation — the
/// keeper is a resident process, and shutdown is the whole process
/// exiting, not this loop returning.
async fn serve(
    listener: tokio::net::UnixListener,
    host_key: PrivateKey,
    authorized_key: PublicKey,
) -> io::Result<()> {
    let config = Arc::new(Config {
        keys: vec![host_key],
        ..Default::default()
    });
    let mut server = SshServer {
        authorized_key: Arc::new(authorized_key),
    };

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
            if let Err(e) = serve(listener, host_key, authorized_key).await {
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
        let server_handler = SshServer {
            authorized_key: Arc::new(authorized_key),
        };
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

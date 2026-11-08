//! The embedded SSH server (ssh spec, task 6.1), end to end: a real `up`
//! (real `nono`, real fd-passed sockets), a real SSH client
//! connecting over the sandbox's own unix socket and authenticating with
//! the real generated client keypair. See `tests/lifecycle_up.rs` for why
//! this needs `CARGO_BIN_EXE_devcroft` and why each such test lives in
//! its own file/process.
//!
//! **Neutral surface** (`test-runtime-fixture`): socket modes, key
//! authentication and the absence of a TCP listener are claims about the
//! keeper, not a provider. It used to `flox init` an environment purely
//! to get past `up`; it now runs on whichever row is selected.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, client_key_paths, down};
use russh::client;
use russh::keys::key::PrivateKeyWithHashAlg;
use russh::keys::{Algorithm, PrivateKey};
use std::sync::Arc;

struct AcceptAnyServerKey;

impl client::Handler for AcceptAnyServerKey {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

async fn authenticate_over(socket_path: &std::path::Path, key: PrivateKey) -> bool {
    let stream = tokio::net::UnixStream::connect(socket_path).await.unwrap();
    let config = Arc::new(client::Config::default());
    let mut handle = client::connect_stream(config, stream, AcceptAnyServerKey)
        .await
        .unwrap();
    let result = handle
        .authenticate_publickey("devcroft", PrivateKeyWithHashAlg::new(Arc::new(key), None))
        .await
        .unwrap();
    result.success()
}

/// ssh spec's "No TCP exposure" scenario, checked directly off `/proc`
/// rather than depending on `ss`/`lsof`/`netstat` being installed (none
/// are, in this environment): every fd `pid` holds that's a socket is
/// cross-referenced against `/proc/net/tcp{,6}` for a LISTEN-state (`0A`)
/// entry with a matching inode.
fn pid_owns_no_listening_tcp_socket(pid: libc::pid_t) -> bool {
    let listening_inodes: std::collections::HashSet<String> = ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .flat_map(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .skip(1)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let state = fields.get(3)?;
            let inode = fields.get(9)?;
            (*state == "0A").then(|| inode.to_string())
        })
        .collect();
    if listening_inodes.is_empty() {
        return true;
    }

    let fd_dir = format!("/proc/{pid}/fd");
    let Ok(entries) = std::fs::read_dir(&fd_dir) else {
        return true; // process already gone; trivially owns nothing
    };
    for entry in entries.flatten() {
        if let Ok(target) = std::fs::read_link(entry.path())
            && let Some(inode) = target
                .to_str()
                .and_then(|s| s.strip_prefix("socket:["))
                .and_then(|s| s.strip_suffix(']'))
            && listening_inodes.contains(inode)
        {
            return false;
        }
    }
    true
}

#[test]
fn ssh_server_authenticates_the_real_client_key_and_binds_no_tcp() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("ssh", |fx| {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(check_one_row(fx));
    });
}

async fn check_one_row(fx: &mut dyn common::ProviderFixture) {
    let sandbox_name = fx.sandbox_name().to_string();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);

    // `up` itself blocks on the keeper's control socket becoming
    // responsive (lifecycle::up's `wait_until_responsive`); the ssh
    // socket comes up on the same fd-passing path, moments earlier in
    // the keeper's own startup, so by the time `up` returns it's ready
    // too — no separate polling needed here.
    assert_eq!(
        fx.bring_up(&UpOptions::default())
            .unwrap_or_else(|e| panic!("row {}: up failed: {e}", fx.name())),
        UpOutcome::Started,
        "row {}",
        fx.name()
    );

    let root_mode = std::fs::metadata(&paths.root).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(root_mode.mode() & 0o777, 0o700, "state dir must be 0700");
    let socket_mode = std::fs::metadata(&paths.ssh_socket).unwrap().permissions();
    assert_eq!(socket_mode.mode() & 0o777, 0o600, "ssh socket must be 0600");
    // The control socket carries the spawn protocol, so it is the more
    // sensitive of the two — yet it was the one left at umask (0755),
    // relying entirely on the 0700 root. Asserted here alongside its
    // sibling so the asymmetry cannot come back.
    let control_mode = std::fs::metadata(&paths.socket).unwrap().permissions();
    assert_eq!(
        control_mode.mode() & 0o777,
        0o600,
        "control socket must be 0600"
    );

    let (client_private_path, _) = client_key_paths().unwrap();
    let client_key = PrivateKey::read_openssh_file(&client_private_path).unwrap();

    assert!(
        authenticate_over(&paths.ssh_socket, client_key).await,
        "the real generated client key must authenticate"
    );

    let unrelated_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    assert!(
        !authenticate_over(&paths.ssh_socket, unrelated_key).await,
        "an unrelated key must be rejected"
    );

    // The pidfile is now "<pid> <start_time>" (state::write_pidfile,
    // added so a resurrected unrelated process at a reused pid can never
    // be mistaken for the recorded one) — only `read_pidfile` itself is
    // public within the crate, so this test (outside it) parses the
    // first token directly rather than reaching for a private function.
    let pid: libc::pid_t = std::fs::read_to_string(&paths.pidfile)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        pid_owns_no_listening_tcp_socket(pid),
        "the keeper must not bind any TCP listening socket"
    );

    down(&sandbox_name).unwrap();
}

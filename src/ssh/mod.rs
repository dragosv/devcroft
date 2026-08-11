//! The `ssh` capability (task group 6): an SSH server embedded in the
//! keeper, reachable only through the sandbox's own unix socket — see
//! CLAUDE.md's "SSH lives inside the boundary, on a unix socket only"
//! invariant and design.md decision 3.

mod config;
mod keys;
mod proxy;
mod server;
mod sftp;

pub use config::{render as render_ssh_config, write_managed_section as write_ssh_config};
pub use keys::{KeyError, ensure_client_keypair, generate_host_key};
pub use proxy::{ProxyError, proxy, sandbox_name_from_host};

use russh::keys::PrivateKey;
use russh::keys::ssh_key::PublicKey;

/// The keeper-side half of task 6.1's key handoff: `up` (host-side,
/// unrestricted) generates the host key and resolves the client's
/// authorized public key, then passes both down as OpenSSH-PEM text over
/// `DEVCROFT_SSH_HOST_KEY`/`DEVCROFT_SSH_AUTHORIZED_KEY` — the keeper
/// cannot read either back off disk itself, since both live under
/// `policy::DEVCROFT_DATA_DIR`, which is baseline-denied even to the
/// keeper's own sandboxed process. Starts the server on its own thread
/// and returns immediately; failures are logged (to the keeper's own
/// stderr, which `up` already redirects to `<state>/<name>/keeper.log`)
/// rather than propagated, so a broken ssh handoff degrades to "no ssh
/// for this sandbox" instead of taking the whole keeper down — exec/shell
/// must keep working regardless.
pub fn start_from_env(listener: std::os::unix::net::UnixListener) {
    let host_key_pem = match std::env::var("DEVCROFT_SSH_HOST_KEY") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("devcroft: ssh: DEVCROFT_SSH_HOST_KEY not set; ssh server disabled");
            return;
        }
    };
    let authorized_key_pem = match std::env::var("DEVCROFT_SSH_AUTHORIZED_KEY") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("devcroft: ssh: DEVCROFT_SSH_AUTHORIZED_KEY not set; ssh server disabled");
            return;
        }
    };
    let host_key = match PrivateKey::from_openssh(&host_key_pem) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("devcroft: ssh: parsing host key: {e}");
            return;
        }
    };
    let authorized_key = match PublicKey::from_openssh(&authorized_key_pem) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("devcroft: ssh: parsing authorized key: {e}");
            return;
        }
    };
    server::spawn(listener, host_key, authorized_key);
}

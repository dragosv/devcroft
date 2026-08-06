//! Client keypair and per-sandbox ephemeral host key generation (ssh
//! spec's "Key management" requirement, task 6.1).

use std::fmt;
use std::io;
use std::path::Path;

use russh::keys::ssh_key::{self, Algorithm, LineEnding, PrivateKey};

#[derive(Debug)]
pub enum KeyError {
    Io(io::Error),
    Format(ssh_key::Error),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Io(e) => write!(f, "{e}"),
            KeyError::Format(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<io::Error> for KeyError {
    fn from(e: io::Error) -> Self {
        KeyError::Io(e)
    }
}

impl From<ssh_key::Error> for KeyError {
    fn from(e: ssh_key::Error) -> Self {
        KeyError::Format(e)
    }
}

/// Generates a fresh ed25519 host key and writes it to `path` (mode 0600,
/// via `write_openssh_file`'s own default). Never reused across `up`s —
/// that's what "ephemeral" means in the ssh spec's "per-sandbox ephemeral
/// host keys stored in each sandbox's state dir": a new one every time,
/// unlike the client keypair below, which is a stable identity. Called
/// host-side, before restriction — the state dir lives under devcroft's
/// own baseline-denied data dir (`policy::DEVCROFT_DATA_DIR`), which the
/// keeper cannot read back once sandboxed, so `up` passes the key
/// material to the keeper directly rather than relying on it re-reading
/// this file.
pub fn generate_host_key(path: &Path) -> Result<PrivateKey, KeyError> {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
    key.write_openssh_file(path, LineEnding::LF)?;
    Ok(key)
}

/// Loads the client ed25519 keypair from `private_path`/`public_path`,
/// generating one on first use (ssh spec's "First run generates keys"
/// scenario: a one-line notice, then proceed). Stable across every `up`
/// and every sandbox — this is devcroft's own identity, authenticated
/// against by every sandbox's keeper — unlike the per-sandbox host key
/// above.
///
/// A file that exists but fails to parse is reported as an error rather
/// than silently regenerated over: that would be a corrupt-key problem
/// worth surfacing, not a "first use" one.
pub fn ensure_client_keypair(
    private_path: &Path,
    public_path: &Path,
) -> Result<PrivateKey, KeyError> {
    if private_path.exists() {
        return Ok(PrivateKey::read_openssh_file(private_path)?);
    }

    if let Some(parent) = private_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    eprintln!(
        "devcroft: generating a new SSH client keypair at {}",
        private_path.display()
    );
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
    key.write_openssh_file(private_path, LineEnding::LF)?;
    key.public_key().write_openssh_file(public_path)?;
    restrict_to_owner(public_path)?;
    Ok(key)
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_host_key_writes_mode_0600_and_returns_matching_key() {
        let dir =
            std::env::temp_dir().join(format!("devcroft-hostkey-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ssh_host_ed25519_key");

        let key = generate_host_key(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.mode() & 0o777, 0o600);

        let reread = PrivateKey::read_openssh_file(&path).unwrap();
        assert_eq!(reread.public_key(), key.public_key());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_client_keypair_generates_once_and_is_stable_thereafter() {
        let dir =
            std::env::temp_dir().join(format!("devcroft-clientkey-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let private_path = dir.join("id_ed25519");
        let public_path = dir.join("id_ed25519.pub");

        let first = ensure_client_keypair(&private_path, &public_path).unwrap();
        let second = ensure_client_keypair(&private_path, &public_path).unwrap();
        assert_eq!(first.public_key(), second.public_key());

        use std::os::unix::fs::PermissionsExt;
        let private_mode = std::fs::metadata(&private_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(private_mode & 0o777, 0o600);
        let public_mode = std::fs::metadata(&public_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(public_mode & 0o777, 0o600);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_client_keypair_reports_a_corrupt_existing_file_rather_than_overwriting() {
        let dir =
            std::env::temp_dir().join(format!("devcroft-clientkey-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let private_path = dir.join("id_ed25519");
        let public_path = dir.join("id_ed25519.pub");
        std::fs::write(&private_path, b"not a key").unwrap();

        assert!(ensure_client_keypair(&private_path, &public_path).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

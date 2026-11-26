//! Per-instance VM SSH key material, `DCSHKEY1` raw-disk bootstrap seed, and
//! hardened SSH client configuration (`add-vm-project-runtime` task 2.4).

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use russh::keys::ssh_key::{Algorithm, LineEnding, PrivateKey};
use serde::{Deserialize, Serialize};

use super::contract::{ErrorLayer, is_valid_hex};

/// Byte offset on the private state disk holding `DCSHKEY1` SSH bootstrap,
/// matching `experiments/microvm/guest-keeper/src/bootstrap.rs`.
pub const BOOTSTRAP_OFFSET: u64 = 8192;
pub const BOOTSTRAP_MAGIC: &[u8; 8] = b"DCSHKEY1";
pub const BOOTSTRAP_MAX_RECORD: usize = 16384;

/// JSON seed written to the private state disk for guest init/SSH bootstrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmSshBootstrapSeed {
    pub schema: u32,
    pub instance_id: String,
    pub host_key: String,
    pub authorized_key: String,
}

/// Host-stored per-instance SSH identity material inside `controller_dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSshKeyMaterial {
    pub client_private_key: PathBuf,
    pub client_public_key: PathBuf,
    pub known_hosts: PathBuf,
    pub host_public_key: String,
    pub bootstrap_seed: VmSshBootstrapSeed,
}

/// Loopback-only local SSH port forward (`-L 127.0.0.1:local:127.0.0.1:guest`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalPortForward {
    pub local_port: u16,
    pub guest_port: u16,
}

#[derive(Debug)]
pub enum VmSshError {
    InvalidSeed(String),
    InvalidForward(String),
    Key(String),
    Io(io::Error),
}

impl VmSshError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            VmSshError::InvalidForward(_) | VmSshError::InvalidSeed(_) => ErrorLayer::Config,
            VmSshError::Key(_) | VmSshError::Io(_) => ErrorLayer::Ssh,
        }
    }
}

impl fmt::Display for VmSshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmSshError::InvalidSeed(msg) => write!(f, "invalid VM SSH bootstrap seed: {msg}"),
            VmSshError::InvalidForward(msg) => write!(f, "invalid local SSH forward: {msg}"),
            VmSshError::Key(msg) => write!(f, "VM SSH key generation failed: {msg}"),
            VmSshError::Io(e) => write!(f, "VM SSH I/O error: {e}"),
        }
    }
}

impl std::error::Error for VmSshError {}

impl From<io::Error> for VmSshError {
    fn from(value: io::Error) -> Self {
        VmSshError::Io(value)
    }
}

impl VmSshBootstrapSeed {
    pub fn validate(&self) -> Result<(), VmSshError> {
        if self.schema != 1
            || !is_valid_hex(&self.instance_id, 32)
            || self.host_key.is_empty()
            || self.authorized_key.is_empty()
        {
            return Err(VmSshError::InvalidSeed(
                "require schema=1, 32-hex instance_id, and non-empty keys".into(),
            ));
        }
        let payload =
            serde_json::to_vec(self).map_err(|e| VmSshError::InvalidSeed(e.to_string()))?;
        if payload.len() > BOOTSTRAP_MAX_RECORD {
            return Err(VmSshError::InvalidSeed(format!(
                "encoded seed {} exceeds {BOOTSTRAP_MAX_RECORD} bytes",
                payload.len()
            )));
        }
        Ok(())
    }

    pub fn write_to_disk(&self, disk_path: &Path) -> Result<(), VmSshError> {
        self.validate()?;
        let payload =
            serde_json::to_vec(self).map_err(|e| VmSshError::InvalidSeed(e.to_string()))?;
        let len_be = (payload.len() as u32).to_be_bytes();
        let mut disk = OpenOptions::new().read(true).write(true).open(disk_path)?;
        disk.seek(SeekFrom::Start(BOOTSTRAP_OFFSET))?;
        disk.write_all(BOOTSTRAP_MAGIC)?;
        disk.write_all(&len_be)?;
        disk.write_all(&payload)?;
        disk.sync_data()?;
        Ok(())
    }

    pub fn read_from_disk(disk_path: &Path) -> Result<Option<Self>, VmSshError> {
        let mut disk = fs::File::open(disk_path)?;
        disk.seek(SeekFrom::Start(BOOTSTRAP_OFFSET))?;
        let mut header = [0u8; 12];
        disk.read_exact(&mut header)?;
        if header == [0u8; 12] {
            return Ok(None);
        }
        if &header[..8] != BOOTSTRAP_MAGIC {
            return Err(VmSshError::InvalidSeed("unknown bootstrap magic".into()));
        }
        let len = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
        if !(1..=BOOTSTRAP_MAX_RECORD).contains(&len) {
            return Err(VmSshError::InvalidSeed(format!(
                "record length {len} outside 1..={BOOTSTRAP_MAX_RECORD}"
            )));
        }
        let mut buf = vec![0u8; len];
        disk.read_exact(&mut buf)?;
        let seed: Self = serde_json::from_slice(&buf)
            .map_err(|e| VmSshError::InvalidSeed(format!("corrupt JSON payload: {e}")))?;
        seed.validate()?;
        Ok(Some(seed))
    }
}

impl VmSshKeyMaterial {
    /// Generate distinct per-instance host and client Ed25519 keys, keeping the
    /// client private key and pinned `known_hosts` inside `controller_dir`
    /// (mode `0600`).
    pub fn generate(
        controller_dir: &Path,
        host_alias: &str,
        instance_id: &str,
    ) -> Result<Self, VmSshError> {
        if !is_valid_hex(instance_id, 32) {
            return Err(VmSshError::InvalidSeed(
                "instance_id must be 32 lowercase hex chars".into(),
            ));
        }
        if host_alias.trim().is_empty() || host_alias.chars().any(char::is_whitespace) {
            return Err(VmSshError::InvalidSeed(
                "host_alias must be non-empty without whitespace".into(),
            ));
        }
        fs::create_dir_all(controller_dir)?;
        fs::set_permissions(controller_dir, fs::Permissions::from_mode(0o700))?;

        let host_priv = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .map_err(|e| VmSshError::Key(e.to_string()))?;
        let host_priv_openssh = host_priv
            .to_openssh(LineEnding::LF)
            .map_err(|e| VmSshError::Key(e.to_string()))?
            .to_string();
        let host_pub_openssh = host_priv
            .public_key()
            .to_openssh()
            .map_err(|e| VmSshError::Key(e.to_string()))?;

        let client_priv = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .map_err(|e| VmSshError::Key(e.to_string()))?;
        let client_private_key = controller_dir.join("id_ed25519");
        let client_public_key = controller_dir.join("id_ed25519.pub");
        let known_hosts = controller_dir.join("known_hosts");

        client_priv
            .write_openssh_file(&client_private_key, LineEnding::LF)
            .map_err(|e| VmSshError::Key(e.to_string()))?;
        client_priv
            .public_key()
            .write_openssh_file(&client_public_key)
            .map_err(|e| VmSshError::Key(e.to_string()))?;
        fs::set_permissions(&client_private_key, fs::Permissions::from_mode(0o600))?;
        fs::set_permissions(&client_public_key, fs::Permissions::from_mode(0o600))?;

        let known_hosts_line = format!("{host_alias} {host_pub_openssh}\n");
        fs::write(&known_hosts, known_hosts_line)?;
        fs::set_permissions(&known_hosts, fs::Permissions::from_mode(0o600))?;

        let client_pub_openssh = client_priv
            .public_key()
            .to_openssh()
            .map_err(|e| VmSshError::Key(e.to_string()))?;
        let seed = VmSshBootstrapSeed {
            schema: 1,
            instance_id: instance_id.to_owned(),
            host_key: host_priv_openssh,
            authorized_key: client_pub_openssh,
        };
        seed.validate()?;

        Ok(Self {
            client_private_key,
            client_public_key,
            known_hosts,
            host_public_key: host_pub_openssh,
            bootstrap_seed: seed,
        })
    }
}

/// Render a hardened `Host <alias>` block pinning the instance host key and
/// explicitly disabling agent, X11, local command, and remote forwarding.
pub fn render_vm_ssh_config_host(
    host_alias: &str,
    proxy_command: &str,
    identity_file: &Path,
    known_hosts: &Path,
) -> String {
    format!(
        "Host {host_alias}\n  \
         ProxyCommand {proxy_command}\n  \
         IdentityFile {}\n  \
         IdentitiesOnly yes\n  \
         StrictHostKeyChecking yes\n  \
         UserKnownHostsFile {}\n  \
         ForwardAgent no\n  \
         ForwardX11 no\n  \
         ForwardX11Trusted no\n  \
         PermitLocalCommand no\n  \
         ClearAllForwardings yes\n",
        identity_file.display(),
        known_hosts.display(),
    )
}

/// Build hardened `ssh` invocation arguments preventing ambient `~/.ssh/config`,
/// agent/X11/remote forwarding, and permitting only explicit loopback `-L`
/// forwards.
pub fn build_vm_ssh_client_args(
    host_alias: &str,
    proxy_command: &str,
    identity_file: &Path,
    known_hosts: &Path,
    forwards: &[LocalPortForward],
) -> Result<Vec<String>, VmSshError> {
    if host_alias.trim().is_empty() || proxy_command.trim().is_empty() {
        return Err(VmSshError::InvalidForward(
            "host_alias and proxy_command must be non-empty".into(),
        ));
    }
    let mut args = vec![
        "-F".into(),
        "/dev/null".into(),
        "-i".into(),
        identity_file.display().to_string(),
        "-o".into(),
        format!("ProxyCommand={proxy_command}"),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", known_hosts.display()),
        "-o".into(),
        "ForwardAgent=no".into(),
        "-o".into(),
        "ForwardX11=no".into(),
        "-o".into(),
        "ForwardX11Trusted=no".into(),
        "-o".into(),
        "PermitLocalCommand=no".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
    ];
    for fwd in forwards {
        if fwd.local_port == 0 || fwd.guest_port == 0 {
            return Err(VmSshError::InvalidForward(
                "local_port and guest_port must be non-zero".into(),
            ));
        }
        args.push("-L".into());
        args.push(format!(
            "127.0.0.1:{}:127.0.0.1:{}",
            fwd.local_port, fwd.guest_port
        ));
    }
    args.push(host_alias.to_owned());
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_distinct_per_instance_keys_and_writes_bootstrap_seed() {
        let base = std::env::temp_dir().join(format!("dc-vm-ssh-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let ctrl_a = base.join("ctrl-a");
        let ctrl_b = base.join("ctrl-b");

        let mat_a = VmSshKeyMaterial::generate(
            &ctrl_a,
            "vm-a.devcroft",
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let mat_b = VmSshKeyMaterial::generate(
            &ctrl_b,
            "vm-b.devcroft",
            "fedcba9876543210fedcba9876543210",
        )
        .unwrap();

        assert_ne!(mat_a.host_public_key, mat_b.host_public_key);
        assert_ne!(
            mat_a.bootstrap_seed.authorized_key,
            mat_b.bootstrap_seed.authorized_key
        );
        assert_eq!(
            fs::metadata(&mat_a.client_private_key)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&mat_a.known_hosts)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let kh = fs::read_to_string(&mat_a.known_hosts).unwrap();
        assert!(kh.starts_with("vm-a.devcroft "));
        assert!(kh.contains(&mat_a.host_public_key));

        let disk = base.join("state.img");
        fs::write(&disk, vec![0u8; 65536]).unwrap();
        assert_eq!(VmSshBootstrapSeed::read_from_disk(&disk).unwrap(), None);

        mat_a.bootstrap_seed.write_to_disk(&disk).unwrap();
        let recovered = VmSshBootstrapSeed::read_from_disk(&disk).unwrap().unwrap();
        assert_eq!(recovered, mat_a.bootstrap_seed);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn hardened_config_and_args_disable_forwarding_risks_and_validate_ports() {
        let id = Path::new("/ctrl/id_ed25519");
        let kh = Path::new("/ctrl/known_hosts");
        let cfg = render_vm_ssh_config_host("alpha.devcroft", "devcroft vm-proxy alpha", id, kh);
        for required in [
            "StrictHostKeyChecking yes",
            "IdentitiesOnly yes",
            "ForwardAgent no",
            "ForwardX11 no",
            "ForwardX11Trusted no",
            "PermitLocalCommand no",
            "ClearAllForwardings yes",
        ] {
            assert!(cfg.contains(required), "missing `{required}` in:\n{cfg}");
        }

        let args = build_vm_ssh_client_args(
            "alpha.devcroft",
            "devcroft vm-proxy alpha",
            id,
            kh,
            &[LocalPortForward {
                local_port: 19090,
                guest_port: 9090,
            }],
        )
        .unwrap();
        assert!(args.windows(2).any(|w| w == ["-F", "/dev/null"]));
        assert!(args.contains(&"ForwardAgent=no".to_string()));
        assert!(args.contains(&"ForwardX11=no".to_string()));
        assert!(args.contains(&"ForwardX11Trusted=no".to_string()));
        assert!(args.contains(&"127.0.0.1:19090:127.0.0.1:9090".to_string()));
        assert!(!args.contains(&"-R".to_string()));

        assert!(matches!(
            build_vm_ssh_client_args(
                "alpha.devcroft",
                "devcroft vm-proxy alpha",
                id,
                kh,
                &[LocalPortForward {
                    local_port: 0,
                    guest_port: 8080,
                }],
            ),
            Err(VmSshError::InvalidForward(_))
        ));
    }
}

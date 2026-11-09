use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const REVISION: &str = "6151710f4ea627cf0e5dbce78e4192fb4d64c2e9";
pub const CONTROL_PORT: u32 = 1024;
pub const KEEPER_PORT: u32 = 1025;
pub const SSH_PORT: u32 = 1026;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub schema: u32,
    pub kernel: Artifact,
    pub initramfs: Artifact,
    pub runtime: Artifact,
    pub workload: Option<Artifact>,
    pub data: Option<PathBuf>,
    pub state: PathBuf,
    pub instance_dir: PathBuf,
    pub vcpus: u8,
    pub memory_mib: u32,
}

impl Launch {
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|e| format!("config: {}: {e}", path.display()))?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("config: read: {e}"))?;
        if bytes.len() > 65536 {
            return Err("config: launch record exceeds 64 KiB".into());
        }
        let mut cfg: Self = serde_json::from_slice(&bytes)
            .map_err(|e| format!("config: invalid launch record: {e}"))?;
        if cfg.schema != 1
            || !(1..=8).contains(&cfg.vcpus)
            || !(128..=4096).contains(&cfg.memory_mib)
        {
            return Err("config: require schema 1, 1..8 vCPUs and 128..4096 MiB".into());
        }
        for artifact in [&mut cfg.kernel, &mut cfg.initramfs, &mut cfg.runtime] {
            artifact.path = regular_file(&artifact.path)?;
            verify(artifact)?;
        }
        if cfg.workload.is_some() != cfg.data.is_some() {
            return Err(
                "config: workload image and private data disk must be provided together".into(),
            );
        }
        if let Some(artifact) = &mut cfg.workload {
            artifact.path = regular_file(&artifact.path)?;
            verify(artifact)?;
        }
        cfg.state = regular_file(&cfg.state)?;
        if let Some(data) = &mut cfg.data {
            *data = regular_file(data)?;
            if *data == cfg.state {
                return Err("config: private data disk aliases state".into());
            }
        }
        for private in std::iter::once(&cfg.state).chain(cfg.data.iter()) {
            let state_meta = fs::metadata(private).map_err(|e| e.to_string())?;
            if state_meta.len() < 8192 || state_meta.nlink() != 1 {
                return Err(
                    "config: private disk must have one link and at least 8192 bytes".into(),
                );
            }
            for artifact in cfg.artifacts() {
                let meta = fs::metadata(&artifact.path).map_err(|e| e.to_string())?;
                if (meta.dev(), meta.ino()) == (state_meta.dev(), state_meta.ino()) {
                    return Err("config: private disk aliases an immutable input".into());
                }
            }
        }
        cfg.instance_dir = fs::canonicalize(&cfg.instance_dir)
            .map_err(|e| format!("config: instance directory: {e}"))?;
        let dir = fs::metadata(&cfg.instance_dir).map_err(|e| e.to_string())?;
        if !dir.is_dir() || dir.uid() != unsafe { libc::geteuid() } || dir.mode() & 0o077 != 0 {
            return Err(
                "config: instance directory must be owned by this user with mode 0700".into(),
            );
        }
        for disk in std::iter::once(&cfg.state).chain(cfg.data.iter()) {
            if disk.parent() != Some(cfg.instance_dir.as_path()) {
                return Err(
                    "config: private disks must be directly inside their instance directory".into(),
                );
            }
        }
        for artifact in cfg.artifacts() {
            if artifact.path.starts_with(&cfg.instance_dir) {
                return Err(
                    "config: immutable input must be outside the writable instance directory"
                        .into(),
                );
            }
        }
        for socket in [cfg.control_socket(), cfg.keeper_socket(), cfg.ssh_socket()] {
            let occupied = match socket.symlink_metadata() {
                Ok(_) => true,
                Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                Err(e) => return Err(format!("config: socket metadata: {e}")),
            };
            if socket.as_os_str().len() > 100 || occupied {
                return Err("config: socket is occupied or its path exceeds 100 bytes".into());
            }
        }
        Ok(cfg)
    }

    pub fn control_socket(&self) -> PathBuf {
        self.instance_dir.join("control.sock")
    }

    pub fn artifacts(&self) -> impl Iterator<Item = &Artifact> {
        [&self.kernel, &self.initramfs, &self.runtime]
            .into_iter()
            .chain(self.workload.iter())
    }

    pub fn keeper_socket(&self) -> PathBuf {
        self.instance_dir.join("keeper.sock")
    }

    pub fn ssh_socket(&self) -> PathBuf {
        self.instance_dir.join("ssh.sock")
    }
}

fn regular_file(path: &Path) -> Result<PathBuf, String> {
    let path = fs::canonicalize(path).map_err(|e| format!("config: {}: {e}", path.display()))?;
    let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(format!(
            "config: {} is not a nonempty regular file",
            path.display()
        ));
    }
    Ok(path)
}

pub fn verify(artifact: &Artifact) -> Result<(), String> {
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("config: invalid SHA-256".into());
    }
    let mut hasher = Sha256::new();
    let mut file = File::open(&artifact.path).map_err(|e| format!("config: input: {e}"))?;
    io::copy(&mut file, &mut hasher).map_err(|e| format!("config: hashing: {e}"))?;
    let actual = format!("{:x}", hasher.finalize());
    if actual != artifact.sha256.to_ascii_lowercase() {
        return Err(format!(
            "config: SHA-256 mismatch: {}",
            artifact.path.display()
        ));
    }
    Ok(())
}

//! Linux guest init mounts, identity/handshake verification, and PID 1
//! quiesced stop acknowledgement (`add-vm-project-runtime` task 2.2).

use std::fmt;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::contract::{ContractError, ErrorLayer, HandshakeV1, LaunchRecordV1};
use super::transport::{MAX_CONTROL_LINE, TransportError, VmControlClient};

pub const HANDSHAKE_COMMAND: &str = "HANDSHAKE";
pub const READY_PREFIX: &str = "READY ";

/// One mount point configured by guest PID 1 before starting keeper/SSH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestMount {
    pub target: String,
    pub fstype: String,
    pub read_only: bool,
    pub options: Vec<String>,
}

/// Expected Linux guest init filesystem layout (raw block disks, zero host shares).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestInitPlanV1 {
    pub mounts: Vec<GuestMount>,
    pub has_nix_store_link: bool,
}

impl GuestInitPlanV1 {
    pub fn standard_linux(has_nix_store_link: bool) -> Self {
        Self {
            mounts: vec![
                GuestMount {
                    target: "/proc".into(),
                    fstype: "proc".into(),
                    read_only: false,
                    options: vec!["nosuid".into(), "nodev".into(), "noexec".into()],
                },
                GuestMount {
                    target: "/sys".into(),
                    fstype: "sysfs".into(),
                    read_only: false,
                    options: vec!["nosuid".into(), "nodev".into(), "noexec".into()],
                },
                GuestMount {
                    target: "/dev".into(),
                    fstype: "devtmpfs".into(),
                    read_only: false,
                    options: vec!["nosuid".into()],
                },
                GuestMount {
                    target: "/dev/pts".into(),
                    fstype: "devpts".into(),
                    read_only: false,
                    options: vec!["nosuid".into(), "noexec".into()],
                },
                GuestMount {
                    target: "/run".into(),
                    fstype: "tmpfs".into(),
                    read_only: false,
                    options: vec!["nosuid".into(), "nodev".into(), "mode=0755".into()],
                },
                GuestMount {
                    target: "/tmp".into(),
                    fstype: "tmpfs".into(),
                    read_only: false,
                    options: vec!["nosuid".into(), "nodev".into(), "mode=1777".into()],
                },
                GuestMount {
                    target: "/toolchain".into(),
                    fstype: "ext4".into(),
                    read_only: true,
                    options: vec!["ro".into(), "nodev".into()],
                },
                GuestMount {
                    target: "/workspace".into(),
                    fstype: "ext4".into(),
                    read_only: false,
                    options: vec!["rw".into(), "nodev".into(), "nosuid".into()],
                },
            ],
            has_nix_store_link,
        }
    }

    pub fn validate(&self) -> Result<(), GuestInitError> {
        let mut saw_proc = false;
        let mut saw_workspace = false;
        for m in &self.mounts {
            let fs_lower = m.fstype.to_ascii_lowercase();
            if fs_lower == "virtiofs" || fs_lower == "9p" {
                return Err(GuestInitError::ForbiddenMount(format!(
                    "host filesystem share `{}` on `{}` is forbidden",
                    m.fstype, m.target
                )));
            }
            if m.target == "/toolchain" && !m.read_only {
                return Err(GuestInitError::ForbiddenMount(
                    "`/toolchain` must be mounted read-only".into(),
                ));
            }
            if m.target == "/proc" {
                saw_proc = true;
            }
            if m.target == "/workspace" {
                saw_workspace = true;
            }
        }
        if !saw_proc || !saw_workspace {
            return Err(GuestInitError::ForbiddenMount(
                "guest init mount plan requires both `/proc` and `/workspace`".into(),
            ));
        }
        Ok(())
    }
}

/// Ordered PID 1 shutdown quiesce steps executed prior to `SYNCED` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopQuiescePlan {
    pub term_grace_secs: u64,
    pub unmount_targets: Vec<String>,
    pub require_all_unmounts: bool,
}

impl StopQuiescePlan {
    pub fn default_plan() -> Self {
        Self {
            term_grace_secs: 5,
            unmount_targets: vec!["/workspace".into(), "/toolchain".into()],
            require_all_unmounts: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestInitError {
    ForbiddenMount(String),
    HandshakeFormat(String),
    HandshakeVerification(ContractError),
    StopRejected(String),
    Timeout(String),
    Transport(String),
}

impl GuestInitError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            GuestInitError::ForbiddenMount(_) => ErrorLayer::Config,
            GuestInitError::HandshakeVerification(e) => e.layer(),
            _ => ErrorLayer::Keeper,
        }
    }
}

impl fmt::Display for GuestInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuestInitError::ForbiddenMount(msg) => write!(f, "invalid guest mount plan: {msg}"),
            GuestInitError::HandshakeFormat(msg) => write!(f, "invalid guest handshake: {msg}"),
            GuestInitError::HandshakeVerification(e) => {
                write!(f, "guest handshake verification failed: {e}")
            }
            GuestInitError::StopRejected(msg) => write!(f, "guest stop verification failed: {msg}"),
            GuestInitError::Timeout(msg) => write!(f, "guest readiness timeout: {msg}"),
            GuestInitError::Transport(msg) => write!(f, "guest control transport error: {msg}"),
        }
    }
}

impl std::error::Error for GuestInitError {}

impl From<ContractError> for GuestInitError {
    fn from(value: ContractError) -> Self {
        GuestInitError::HandshakeVerification(value)
    }
}

impl From<TransportError> for GuestInitError {
    fn from(value: TransportError) -> Self {
        GuestInitError::Transport(value.to_string())
    }
}

pub fn render_ready_handshake(handshake: &HandshakeV1) -> Result<String, GuestInitError> {
    let body = serde_json::to_string(handshake)
        .map_err(|e| GuestInitError::HandshakeFormat(e.to_string()))?;
    let line = format!("{READY_PREFIX}{body}");
    if line.len() > MAX_CONTROL_LINE {
        return Err(GuestInitError::HandshakeFormat(
            "rendered handshake exceeds MAX_CONTROL_LINE".into(),
        ));
    }
    Ok(line)
}

pub fn parse_and_verify_ready_handshake(
    line: &str,
    launch: &LaunchRecordV1,
    expected_protocol: u32,
) -> Result<HandshakeV1, GuestInitError> {
    let raw = line.strip_prefix(READY_PREFIX).ok_or_else(|| {
        GuestInitError::HandshakeFormat(format!("expected `{READY_PREFIX}<json>`, got `{line}`"))
    })?;
    let hs: HandshakeV1 =
        serde_json::from_str(raw).map_err(|e| GuestInitError::HandshakeFormat(e.to_string()))?;
    hs.validate_against(launch, expected_protocol)?;
    Ok(hs)
}

/// Poll `control_socket` until the guest init answers `HANDSHAKE`, verifying
/// generation, protocol version, guest target, boot artifact SHA-256, and
/// required capabilities before exposing sessions.
pub fn wait_for_verified_handshake(
    control_socket: &Path,
    launch: &LaunchRecordV1,
    expected_protocol: u32,
    timeout: Duration,
) -> Result<HandshakeV1, GuestInitError> {
    launch.validate()?;
    let deadline = Instant::now() + timeout;
    let client = VmControlClient::new(control_socket, timeout);

    let reply = loop {
        match client.send_command(HANDSHAKE_COMMAND) {
            Ok(line) => break line,
            Err(TransportError::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(GuestInitError::Timeout(format!(
                        "timed out waiting for control socket {}: {e}",
                        control_socket.display()
                    )));
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(other) => return Err(GuestInitError::from(other)),
        }
    };

    parse_and_verify_ready_handshake(&reply, launch, expected_protocol)
}

pub fn format_stop_request(generation: u64, nonce: &str) -> Result<String, GuestInitError> {
    if generation == 0
        || nonce.is_empty()
        || nonce.chars().any(char::is_whitespace)
        || nonce.len() > 128
    {
        return Err(GuestInitError::StopRejected(
            "invalid generation or nonce for SYNC-STOP".into(),
        ));
    }
    Ok(format!("SYNC-STOP {generation} {nonce}"))
}

/// Require an exact `SYNCED <generation> <nonce>` acknowledgement, rejecting
/// stale replies, `ERR shutdown-busy`, or un-correlated `PONG` responses.
pub fn verify_stop_acknowledgement(
    reply: &str,
    expected_generation: u64,
    expected_nonce: &str,
) -> Result<(), GuestInitError> {
    let trimmed = reply.trim();
    if trimmed == "ERR shutdown-busy" {
        return Err(GuestInitError::StopRejected(
            "guest init reported busy filesystems (`ERR shutdown-busy`)".into(),
        ));
    }
    let expected = format!("SYNCED {expected_generation} {expected_nonce}");
    if trimmed != expected {
        return Err(GuestInitError::StopRejected(format!(
            "unexpected stop reply `{trimmed}`; expected `{expected}`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::contract::{
        CONTRACT_SCHEMA_V1, DiskAttachment, GuestPhase, GuestTarget, HostChannels, ResourceBudgets,
        VmAdapter,
    };
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample_launch() -> LaunchRecordV1 {
        LaunchRecordV1 {
            schema: CONTRACT_SCHEMA_V1,
            instance_generation: 11,
            target: GuestTarget::LinuxArm64,
            adapter: VmAdapter::Libkrun,
            phase: GuestPhase::Runtime,
            boot_artifact_sha256: SHA_A.into(),
            required_capabilities: vec!["nono".into(), "ssh".into()],
            disks: vec![DiskAttachment {
                id: "workspace".into(),
                path: "/inst/workspace.img".into(),
                read_only: false,
                sha256: None,
            }],
            channels: HostChannels {
                control_socket: "/inst/control.sock".into(),
                keeper_socket: "/inst/keeper.sock".into(),
                ssh_socket: "/inst/ssh.sock".into(),
                proxy_socket: None,
                allow_nic: false,
                allow_tsi: false,
            },
            resources: ResourceBudgets {
                vcpus: 2,
                guest_memory_mib: 512,
                writable_disk_mib: 1024,
                log_bytes_cap: 1024 * 1024,
                readiness_timeout_secs: 30,
            },
        }
    }

    #[test]
    fn mount_plan_allows_raw_ext4_and_rejects_virtiofs_or_writable_toolchain() {
        let plan = GuestInitPlanV1::standard_linux(true);
        plan.validate().unwrap();
        assert_eq!(
            StopQuiescePlan::default_plan().unmount_targets,
            vec!["/workspace", "/toolchain"]
        );

        let mut with_virtiofs = plan.clone();
        with_virtiofs.mounts.push(GuestMount {
            target: "/host".into(),
            fstype: "virtiofs".into(),
            read_only: false,
            options: vec![],
        });
        assert!(matches!(
            with_virtiofs.validate(),
            Err(GuestInitError::ForbiddenMount(_))
        ));

        let mut rw_toolchain = plan;
        rw_toolchain
            .mounts
            .iter_mut()
            .find(|m| m.target == "/toolchain")
            .unwrap()
            .read_only = false;
        assert!(matches!(
            rw_toolchain.validate(),
            Err(GuestInitError::ForbiddenMount(_))
        ));
    }

    #[test]
    fn handshake_wait_verifies_generation_artifact_target_and_caps() {
        let dir = std::env::temp_dir().join(format!("dc-vm-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        let launch = sample_launch();
        let valid_hs = HandshakeV1 {
            schema: CONTRACT_SCHEMA_V1,
            instance_generation: 11,
            protocol_version: 1,
            target: GuestTarget::LinuxArm64,
            boot_artifact_sha256: SHA_A.into(),
            capabilities: vec!["nono".into(), "ssh".into()],
        };
        let valid_line = render_ready_handshake(&valid_hs).unwrap();
        let mut stale_hs = valid_hs;
        stale_hs.instance_generation = 10;
        let stale_line = render_ready_handshake(&stale_hs).unwrap();

        thread::spawn(move || {
            for (idx, conn) in listener.incoming().take(2).flatten().enumerate() {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut req = String::new();
                let _ = reader.read_line(&mut req);
                let mut w = conn;
                let resp = if idx == 0 { &valid_line } else { &stale_line };
                let _ = w.write_all(resp.as_bytes());
                let _ = w.write_all(b"\n");
            }
        });

        let ok = wait_for_verified_handshake(&sock, &launch, 1, Duration::from_secs(2)).unwrap();
        assert_eq!(ok.instance_generation, 11);

        let err =
            wait_for_verified_handshake(&sock, &launch, 1, Duration::from_secs(2)).unwrap_err();
        assert!(matches!(
            err,
            GuestInitError::HandshakeVerification(ContractError::GenerationMismatch {
                expected: 11,
                found: 10
            })
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_acknowledgement_enforces_exact_generation_and_nonce() {
        let req = format_stop_request(42, "nonce-abc").unwrap();
        assert_eq!(req, "SYNC-STOP 42 nonce-abc");

        verify_stop_acknowledgement("SYNCED 42 nonce-abc\n", 42, "nonce-abc").unwrap();
        assert!(verify_stop_acknowledgement("ERR shutdown-busy", 42, "nonce-abc").is_err());
        assert!(verify_stop_acknowledgement("PONG", 42, "nonce-abc").is_err());
        assert!(verify_stop_acknowledgement("SYNCED 41 nonce-abc", 42, "nonce-abc").is_err());
        assert!(verify_stop_acknowledgement("SYNCED 42 wrong", 42, "nonce-abc").is_err());
    }
}

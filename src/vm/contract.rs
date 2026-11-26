//! Versioned launch, handshake, state, health, and error-layer contracts
//! for the opt-in VM project runtime (`add-vm-project-runtime` task 1.2).

use std::fmt;

use serde::{Deserialize, Serialize};

pub const CONTRACT_SCHEMA_V1: u32 = 1;

/// Guest OS + architecture target for a VM runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuestTarget {
    LinuxArm64,
    LinuxX86_64,
}

impl GuestTarget {
    pub const fn as_str(&self) -> &'static str {
        match self {
            GuestTarget::LinuxArm64 => "linux-arm64",
            GuestTarget::LinuxX86_64 => "linux-x86_64",
        }
    }
}

/// VMM adapter selected by the accepted evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VmAdapter {
    Libkrun,
}

impl VmAdapter {
    pub const fn as_str(&self) -> &'static str {
        match self {
            VmAdapter::Libkrun => "libkrun",
        }
    }
}

/// Host-tracked execution phase for a VM instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuestPhase {
    Preparation,
    Setup,
    Runtime,
}

/// Runtime kind recorded for an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RuntimeKind {
    Native,
    Vm {
        target: GuestTarget,
        adapter: VmAdapter,
    },
}

/// Who chose the active runtime (`status` must surface this explicitly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeSelectionSource {
    UserGrant,
    ExplicitOptIn,
    QualifiedVmDefault,
    ExistingInstance,
    OptInDefault,
}

impl RuntimeSelectionSource {
    pub const fn describe(&self) -> &'static str {
        match self {
            RuntimeSelectionSource::UserGrant => "user-owned native runtime grant",
            RuntimeSelectionSource::ExplicitOptIn => "explicit user VM opt-in",
            RuntimeSelectionSource::QualifiedVmDefault => {
                "qualified VM row (no native grant for project)"
            }
            RuntimeSelectionSource::ExistingInstance => "preserved existing instance runtime",
            RuntimeSelectionSource::OptInDefault => "default native tier (VM remains opt-in)",
        }
    }
}

/// Existing devcroft error layers and their documented CLI exit codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorLayer {
    Config,
    Provider,
    Backend,
    Keeper,
    Ssh,
}

impl ErrorLayer {
    pub const fn exit_code(&self) -> u8 {
        match self {
            ErrorLayer::Config => 2,
            ErrorLayer::Provider => 3,
            ErrorLayer::Backend => 4,
            ErrorLayer::Keeper => 5,
            ErrorLayer::Ssh => 5,
        }
    }

    pub const fn layer_name(&self) -> &'static str {
        match self {
            ErrorLayer::Config => "config",
            ErrorLayer::Provider => "provider",
            ErrorLayer::Backend => "backend",
            ErrorLayer::Keeper => "keeper",
            ErrorLayer::Ssh => "ssh",
        }
    }
}

/// One raw disk attached to the VM helper (immutable input or private RW).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskAttachment {
    pub id: String,
    pub path: String,
    pub read_only: bool,
    pub sha256: Option<String>,
}

/// Explicitly bounded host–guest endpoints (no NIC or TSI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostChannels {
    pub control_socket: String,
    pub keeper_socket: String,
    pub ssh_socket: String,
    pub proxy_socket: Option<String>,
    pub allow_nic: bool,
    pub allow_tsi: bool,
}

/// Finite guest and host-budget constraints verified at launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceBudgets {
    pub vcpus: u32,
    pub guest_memory_mib: u32,
    pub writable_disk_mib: u32,
    pub log_bytes_cap: u64,
    pub readiness_timeout_secs: u64,
}

/// Trusted controller launch record passed to the isolated VMM helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRecordV1 {
    pub schema: u32,
    pub instance_generation: u64,
    pub target: GuestTarget,
    pub adapter: VmAdapter,
    pub phase: GuestPhase,
    pub boot_artifact_sha256: String,
    pub required_capabilities: Vec<String>,
    pub disks: Vec<DiskAttachment>,
    pub channels: HostChannels,
    pub resources: ResourceBudgets,
}

/// Guest-to-controller handshake verified before exposing sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeV1 {
    pub schema: u32,
    pub instance_generation: u64,
    pub protocol_version: u32,
    pub target: GuestTarget,
    pub boot_artifact_sha256: String,
    pub capabilities: Vec<String>,
}

/// Process identity stronger than a bare OS PID to prevent PID-reuse errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostProcessIdentity {
    pub pid: u32,
    pub start_time_ns: u64,
}

/// Health of an individual runtime component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentHealth {
    Healthy,
    Unhealthy,
    Stopped,
    Unknown,
}

/// Separated health states so helper liveness never implies project readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmHealthBreakdown {
    pub helper: ComponentHealth,
    pub transport: ComponentHealth,
    pub keeper: ComponentHealth,
    pub environment_fresh: bool,
    pub services: ComponentHealth,
}

impl VmHealthBreakdown {
    /// Sessions require helper, transport, and keeper all to be healthy.
    pub fn is_project_ready(&self) -> bool {
        self.helper == ComponentHealth::Healthy
            && self.transport == ComponentHealth::Healthy
            && self.keeper == ComponentHealth::Healthy
    }
}

/// Outcome of the most recent shutdown sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopClassification {
    Orderly,
    ForcedStop,
    Crash,
    UnverifiedStop,
}

/// Persistent host-side VM instance record (`vm-state.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmStateRecordV1 {
    pub schema: u32,
    pub runtime_kind: RuntimeKind,
    pub selection_source: RuntimeSelectionSource,
    pub instance_generation: u64,
    pub host_process: Option<HostProcessIdentity>,
    pub private_disk_id: String,
    pub boot_artifact_sha256: String,
    pub health: VmHealthBreakdown,
    pub last_stop: Option<StopClassification>,
}

/// Validation or handshake failure attributed to a documented error layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    UnsupportedSchema {
        expected: u32,
        found: u32,
    },
    InvalidGeneration,
    InvalidSha256 {
        field: &'static str,
    },
    ForbiddenNetworkFeature(&'static str),
    InvalidResourceBudget(&'static str),
    GenerationMismatch {
        expected: u64,
        found: u64,
    },
    ProtocolMismatch {
        expected: u32,
        found: u32,
    },
    TargetMismatch {
        expected: GuestTarget,
        found: GuestTarget,
    },
    BootArtifactMismatch,
    MissingCapability(String),
}

impl ContractError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            ContractError::UnsupportedSchema { .. }
            | ContractError::InvalidGeneration
            | ContractError::InvalidSha256 { .. }
            | ContractError::ForbiddenNetworkFeature(_)
            | ContractError::InvalidResourceBudget(_) => ErrorLayer::Config,
            ContractError::GenerationMismatch { .. }
            | ContractError::ProtocolMismatch { .. }
            | ContractError::TargetMismatch { .. }
            | ContractError::BootArtifactMismatch
            | ContractError::MissingCapability(_) => ErrorLayer::Keeper,
        }
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractError::UnsupportedSchema { expected, found } => {
                write!(f, "unsupported schema {found}; expected {expected}")
            }
            ContractError::InvalidGeneration => {
                write!(f, "instance_generation must be non-zero")
            }
            ContractError::InvalidSha256 { field } => {
                write!(
                    f,
                    "{field} must be a 64-character lowercase hex SHA-256 digest"
                )
            }
            ContractError::ForbiddenNetworkFeature(name) => {
                write!(
                    f,
                    "{name} must be disabled (initial runtime forbids NIC and TSI)"
                )
            }
            ContractError::InvalidResourceBudget(field) => {
                write!(f, "resource budget `{field}` must be greater than zero")
            }
            ContractError::GenerationMismatch { expected, found } => {
                write!(
                    f,
                    "stale or mismatched instance generation in handshake: expected {expected}, got {found}"
                )
            }
            ContractError::ProtocolMismatch { expected, found } => {
                write!(
                    f,
                    "incompatible keeper handshake protocol version: expected {expected}, got {found}"
                )
            }
            ContractError::TargetMismatch { expected, found } => {
                write!(
                    f,
                    "guest target mismatch: expected {}, got {}",
                    expected.as_str(),
                    found.as_str()
                )
            }
            ContractError::BootArtifactMismatch => {
                write!(
                    f,
                    "handshake boot artifact identity does not match launch record"
                )
            }
            ContractError::MissingCapability(cap) => {
                write!(f, "guest handshake missing required capability `{cap}`")
            }
        }
    }
}

impl std::error::Error for ContractError {}

pub(crate) fn is_valid_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl LaunchRecordV1 {
    /// Validate invariants that must hold before invoking the VMM helper.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema != CONTRACT_SCHEMA_V1 {
            return Err(ContractError::UnsupportedSchema {
                expected: CONTRACT_SCHEMA_V1,
                found: self.schema,
            });
        }
        if self.instance_generation == 0 {
            return Err(ContractError::InvalidGeneration);
        }
        if !is_valid_hex(&self.boot_artifact_sha256, 64) {
            return Err(ContractError::InvalidSha256 {
                field: "boot_artifact_sha256",
            });
        }
        for disk in &self.disks {
            if let Some(sha) = &disk.sha256
                && !is_valid_hex(sha, 64)
            {
                return Err(ContractError::InvalidSha256 {
                    field: "disks[].sha256",
                });
            }
        }
        if self.channels.allow_nic {
            return Err(ContractError::ForbiddenNetworkFeature("allow_nic"));
        }
        if self.channels.allow_tsi {
            return Err(ContractError::ForbiddenNetworkFeature("allow_tsi"));
        }
        if self.resources.vcpus == 0 {
            return Err(ContractError::InvalidResourceBudget("vcpus"));
        }
        if self.resources.guest_memory_mib == 0 {
            return Err(ContractError::InvalidResourceBudget("guest_memory_mib"));
        }
        if self.resources.writable_disk_mib == 0 {
            return Err(ContractError::InvalidResourceBudget("writable_disk_mib"));
        }
        if self.resources.log_bytes_cap == 0 {
            return Err(ContractError::InvalidResourceBudget("log_bytes_cap"));
        }
        if self.resources.readiness_timeout_secs == 0 {
            return Err(ContractError::InvalidResourceBudget(
                "readiness_timeout_secs",
            ));
        }
        Ok(())
    }
}

impl HandshakeV1 {
    /// Verify that a guest handshake matches the trusted controller launch record.
    pub fn validate_against(
        &self,
        launch: &LaunchRecordV1,
        expected_protocol: u32,
    ) -> Result<(), ContractError> {
        launch.validate()?;
        if self.schema != CONTRACT_SCHEMA_V1 {
            return Err(ContractError::UnsupportedSchema {
                expected: CONTRACT_SCHEMA_V1,
                found: self.schema,
            });
        }
        if self.instance_generation != launch.instance_generation {
            return Err(ContractError::GenerationMismatch {
                expected: launch.instance_generation,
                found: self.instance_generation,
            });
        }
        if self.protocol_version != expected_protocol {
            return Err(ContractError::ProtocolMismatch {
                expected: expected_protocol,
                found: self.protocol_version,
            });
        }
        if self.target != launch.target {
            return Err(ContractError::TargetMismatch {
                expected: launch.target,
                found: self.target,
            });
        }
        if !is_valid_hex(&self.boot_artifact_sha256, 64)
            || self.boot_artifact_sha256 != launch.boot_artifact_sha256
        {
            return Err(ContractError::BootArtifactMismatch);
        }
        for req in &launch.required_capabilities {
            if !self.capabilities.contains(req) {
                return Err(ContractError::MissingCapability(req.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn sample_launch() -> LaunchRecordV1 {
        LaunchRecordV1 {
            schema: CONTRACT_SCHEMA_V1,
            instance_generation: 7,
            target: GuestTarget::LinuxArm64,
            adapter: VmAdapter::Libkrun,
            phase: GuestPhase::Runtime,
            boot_artifact_sha256: SHA_A.to_owned(),
            required_capabilities: vec!["nono".into(), "ssh".into()],
            disks: vec![DiskAttachment {
                id: "runtime".into(),
                path: "/var/lib/devcroft/runtime.img".into(),
                read_only: true,
                sha256: Some(SHA_B.to_owned()),
            }],
            channels: HostChannels {
                control_socket: "/tmp/control.sock".into(),
                keeper_socket: "/tmp/keeper.sock".into(),
                ssh_socket: "/tmp/ssh.sock".into(),
                proxy_socket: Some("/tmp/proxy.sock".into()),
                allow_nic: false,
                allow_tsi: false,
            },
            resources: ResourceBudgets {
                vcpus: 2,
                guest_memory_mib: 512,
                writable_disk_mib: 1024,
                log_bytes_cap: 1024 * 1024,
                readiness_timeout_secs: 60,
            },
        }
    }

    fn sample_handshake() -> HandshakeV1 {
        HandshakeV1 {
            schema: CONTRACT_SCHEMA_V1,
            instance_generation: 7,
            protocol_version: 1,
            target: GuestTarget::LinuxArm64,
            boot_artifact_sha256: SHA_A.to_owned(),
            capabilities: vec!["nono".into(), "ssh".into(), "pty".into()],
        }
    }

    #[test]
    fn launch_handshake_and_state_round_trip_and_validate() {
        let launch = sample_launch();
        launch.validate().unwrap();
        let json = serde_json::to_string(&launch).unwrap();
        let decoded: LaunchRecordV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(launch, decoded);

        let hs = sample_handshake();
        hs.validate_against(&launch, 1).unwrap();

        let state = VmStateRecordV1 {
            schema: CONTRACT_SCHEMA_V1,
            runtime_kind: RuntimeKind::Vm {
                target: GuestTarget::LinuxArm64,
                adapter: VmAdapter::Libkrun,
            },
            selection_source: RuntimeSelectionSource::QualifiedVmDefault,
            instance_generation: 7,
            host_process: Some(HostProcessIdentity {
                pid: 4242,
                start_time_ns: 99_000,
            }),
            private_disk_id: "disk-7".into(),
            boot_artifact_sha256: SHA_A.to_owned(),
            health: VmHealthBreakdown {
                helper: ComponentHealth::Healthy,
                transport: ComponentHealth::Healthy,
                keeper: ComponentHealth::Healthy,
                environment_fresh: true,
                services: ComponentHealth::Stopped,
            },
            last_stop: Some(StopClassification::Orderly),
        };
        assert!(state.health.is_project_ready());
        let state_json = serde_json::to_string(&state).unwrap();
        let state_back: VmStateRecordV1 = serde_json::from_str(&state_json).unwrap();
        assert_eq!(state, state_back);
    }

    #[test]
    fn unknown_fields_in_versioned_records_are_rejected() {
        let launch_json = serde_json::to_value(sample_launch()).unwrap();
        let mut map = launch_json.as_object().unwrap().clone();
        map.insert("unexpected".into(), serde_json::json!(true));
        let err =
            serde_json::from_value::<LaunchRecordV1>(serde_json::Value::Object(map)).unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn handshake_rejects_stale_generation_target_artifact_and_missing_cap() {
        let launch = sample_launch();

        let mut stale = sample_handshake();
        stale.instance_generation = 6;
        assert_eq!(
            stale.validate_against(&launch, 1).unwrap_err(),
            ContractError::GenerationMismatch {
                expected: 7,
                found: 6
            }
        );

        let mut wrong_target = sample_handshake();
        wrong_target.target = GuestTarget::LinuxX86_64;
        assert_eq!(
            wrong_target.validate_against(&launch, 1).unwrap_err(),
            ContractError::TargetMismatch {
                expected: GuestTarget::LinuxArm64,
                found: GuestTarget::LinuxX86_64
            }
        );

        let mut wrong_art = sample_handshake();
        wrong_art.boot_artifact_sha256 = SHA_B.to_owned();
        assert_eq!(
            wrong_art.validate_against(&launch, 1).unwrap_err(),
            ContractError::BootArtifactMismatch
        );

        let mut missing_cap = sample_handshake();
        missing_cap.capabilities = vec!["nono".into()];
        assert_eq!(
            missing_cap.validate_against(&launch, 1).unwrap_err(),
            ContractError::MissingCapability("ssh".into())
        );
    }

    #[test]
    fn launch_rejects_nic_and_tsi_and_helper_liveness_does_not_imply_readiness() {
        let mut with_nic = sample_launch();
        with_nic.channels.allow_nic = true;
        assert_eq!(
            with_nic.validate().unwrap_err(),
            ContractError::ForbiddenNetworkFeature("allow_nic")
        );

        let mut with_tsi = sample_launch();
        with_tsi.channels.allow_tsi = true;
        assert_eq!(
            with_tsi.validate().unwrap_err(),
            ContractError::ForbiddenNetworkFeature("allow_tsi")
        );

        let helper_only = VmHealthBreakdown {
            helper: ComponentHealth::Healthy,
            transport: ComponentHealth::Unknown,
            keeper: ComponentHealth::Stopped,
            environment_fresh: false,
            services: ComponentHealth::Stopped,
        };
        assert!(!helper_only.is_project_ready());

        assert_eq!(ErrorLayer::Config.exit_code(), 2);
        assert_eq!(ErrorLayer::Provider.exit_code(), 3);
        assert_eq!(ErrorLayer::Backend.exit_code(), 4);
        assert_eq!(ErrorLayer::Keeper.exit_code(), 5);
        assert_eq!(ErrorLayer::Ssh.exit_code(), 5);
    }
}

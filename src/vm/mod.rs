//! Phase 1 and Phase 2 host-side VM primitives for `add-vm-project-runtime`.
//!
//! Implemented incrementally following the owner decision on
//! `evaluate-microvm-runtimes` task 4.5 (`docs/decisions.md`, 2026-11-26;
//! `results/macos-arm64-evaluation-assessment-v16.json`). This module contains
//! zero VMM or Git dependencies so the default crate retains its native
//! behavior and crates.io publishability while the separate helper and artifact
//! channel is built out.

pub mod contract;
pub mod editor;
pub mod grants;
pub mod launcher;
pub mod manifest;
pub mod qualification;
pub mod ssh;
pub mod transport;

pub use contract::{
    ComponentHealth, ContractError, DiskAttachment, ErrorLayer, GuestPhase, GuestTarget,
    HandshakeV1, HostChannels, HostProcessIdentity, LaunchRecordV1, ResourceBudgets, RuntimeKind,
    RuntimeSelectionSource, StopClassification, VmAdapter, VmHealthBreakdown, VmStateRecordV1,
};
pub use editor::{
    EditorClientKind, EditorTrustClass, VmSshConfigOutcome, classify_editor,
    guest_served_editor_forward_args, ssh_config_for_vm_instance,
};
pub use grants::{
    ExistingRuntime, GrantError, GrantedRuntime, ProjectIdentity, RuntimeDecision,
    RuntimeDispatchError, RuntimeGrant, RuntimeGrantStore, RuntimePolicyMode, RuntimeRequest,
    resolve_runtime,
};
pub use launcher::{
    HelperArtifactInput, HelperLaunchFileV1, HelperProcess, LaunchError, LogSnapshot,
    VmEndpointPaths, prepare_instance_layout, spawn_helper, write_private_launch_record,
};
pub use manifest::{
    ArtifactEntry, ManifestError, PackagedLayout, TrustedArtifactManifestV1, TrustedArtifactsV1,
};
pub use qualification::{
    HostPlatform, PlatformQualification, QualificationStatus, current_host_platform,
    qualification_for,
};
pub use ssh::{
    LocalPortForward, VmSshBootstrapSeed, VmSshError, VmSshKeyMaterial, build_vm_ssh_client_args,
    render_vm_ssh_config_host,
};
pub use transport::{CommandOutput, TransportError, VmControlClient, VmKeeperClient, VmSession};

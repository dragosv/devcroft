//! Phase 1 contracts, platform qualification, trusted artifact manifest,
//! and user-owned runtime grants for `add-vm-project-runtime`.
//!
//! Implemented per the owner decision on `evaluate-microvm-runtimes` task 4.5
//! (`docs/decisions.md`, 2026-11-26; `results/macos-arm64-evaluation-assessment-v16.json`).
//! This module contains zero VMM or Git dependencies so the default crate
//! retains its native behavior and crates.io publishability while the
//! separate helper and artifact channel is built out.

pub mod contract;
pub mod grants;
pub mod manifest;
pub mod qualification;

pub use contract::{
    ComponentHealth, ContractError, DiskAttachment, ErrorLayer, GuestPhase, GuestTarget,
    HandshakeV1, HostChannels, HostProcessIdentity, LaunchRecordV1, ResourceBudgets, RuntimeKind,
    RuntimeSelectionSource, StopClassification, VmAdapter, VmHealthBreakdown, VmStateRecordV1,
};
pub use grants::{
    ExistingRuntime, GrantError, GrantedRuntime, ProjectIdentity, RuntimeDecision,
    RuntimeDispatchError, RuntimeGrant, RuntimeGrantStore, RuntimePolicyMode, RuntimeRequest,
    resolve_runtime,
};
pub use manifest::{
    ArtifactEntry, ManifestError, PackagedLayout, TrustedArtifactManifestV1, TrustedArtifactsV1,
};
pub use qualification::{
    HostPlatform, PlatformQualification, QualificationStatus, current_host_platform,
    qualification_for,
};

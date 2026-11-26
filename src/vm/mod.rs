//! Phase 1–4 host-side VM runtime primitives for `add-vm-project-runtime`.
//!
//! Implemented incrementally following the owner decision on
//! `evaluate-microvm-runtimes` task 4.5 (`docs/decisions.md`, 2026-11-26;
//! `results/macos-arm64-evaluation-assessment-v16.json`). This module contains
//! zero VMM or Git dependencies so the default crate retains its native
//! behavior and crates.io publishability while the separate helper and artifact
//! channel is built out.

pub mod authority;
pub mod cli_runtime;
pub mod contract;
pub mod credentials;
pub mod editor;
pub mod grants;
pub mod guest_init;
pub mod launcher;
pub mod lifecycle;
pub mod manifest;
pub mod preparation;
pub mod proxy_channel;
pub mod publication;
pub mod qualification;
pub(crate) mod sha256;
pub mod ssh;
pub mod transport;

pub use authority::{
    AuthorityError, AuthorityOrigin, CompiledVmAuthority, EffectiveEgressGrant,
    ExplorationProfileV1, HostResourceEnforcement, HostResourceQualification,
    ProjectAuthorityRequest, UserAuthorityGrants, compile_effective_authority,
    qualify_host_resource_controls, validate_resource_budgets,
};
pub use cli_runtime::{
    DEFAULT_VM_MEMORY_MIB, DEFAULT_VM_VCPUS, InstallLocalVmBundleRequest, InstalledVmBundleSummary,
    LiveControlReply, VM_GUEST_EGRESS_PROXY_PORT, VmBrokerSessionEntry, VmCliError, VmCliInstance,
    VmCliMetadata, VmLiveStatus, VmSyncSummary, VmUpOptions, VmUpOutcome, current_grant_timestamp,
    down_vm_instance, export_vm_disk, inspect_default_vm_bundle, install_local_vm_bundle,
    list_vm_instances, load_vm_instance, logs_vm_instance, map_guest_cwd, probe_live_control,
    render_vm_policy_plan, resolve_vm_manifest_path, rm_vm_instance, send_live_shutdown,
    status_vm_instance, sync_project_into_vm, up_vm_instance, vm_session_env,
    write_vm_ssh_config_section,
};
pub use contract::{
    ComponentHealth, ContractError, DiskAttachment, ErrorLayer, GuestPhase, GuestTarget,
    HandshakeV1, HostChannels, HostProcessIdentity, LaunchRecordV1, ResourceBudgets, RuntimeKind,
    RuntimeSelectionSource, StopClassification, VmAdapter, VmHealthBreakdown, VmStateRecordV1,
};
pub use credentials::{
    CredentialError, DirectCredential, DirectCredentialDelivery, InstanceCredentialState,
    deliver_post_boot_credentials, guard_shared_base_promotion,
    reset_credentials_for_new_instance_or_refresh,
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
pub use guest_init::{
    GuestInitError, GuestInitPlanV1, GuestMount, StopQuiescePlan, format_stop_request,
    parse_and_verify_ready_handshake, render_ready_handshake, verify_stop_acknowledgement,
    wait_for_verified_handshake,
};
pub use launcher::{
    HelperArtifactInput, HelperLaunchFileV1, HelperProcess, LaunchError, LogSnapshot,
    VmEndpointPaths, prepare_instance_layout, spawn_helper, write_private_launch_record,
};
pub use lifecycle::{
    RemovalReport, RestartPlan, ShutdownOutcome, VmInstancePaths, VmLifecycleError,
    VmRollbackPolicy, export_retained_vm_disk, graceful_down, guard_new_vm_creation,
    is_owned_helper_alive, load_vm_state, prepare_cold_restart, probe_vm_health,
    remove_owned_instance, save_vm_state,
};
pub use manifest::{
    ArtifactEntry, ManifestError, PackagedLayout, TrustedArtifactManifestV1, TrustedArtifactsV1,
};
pub use preparation::{
    CANONICAL_GUEST_WORKSPACE, CapturedLinuxProfileV1, HashedInputFile, InputSetPolicy,
    PreparationError, PreparationJobSpecV1, RecipeIdentity, collect_recipe_identity,
    requalify_provider_for_linux,
};
pub use proxy_channel::{
    AdminChannelOrigin, HostPhaseProxy, ProxyChannelError, RefusedDestination,
    SetupRevocationReceipt, SetupWindowStatus, is_forbidden_private_or_local_ip,
};
pub use publication::{
    ArtifactCacheStore, CrossProjectEligibility, GuestOutputEnvelopeV1, PublicationError,
    PublishedArtifactRecordV1, RetentionLeaseV1, StagedPublication,
};
pub use qualification::{
    HostPlatform, PlatformQualification, QualificationStatus, current_host_platform,
    qualification_for,
};
pub use sha256::sha256_file_hex;
pub use ssh::{
    LocalPortForward, VmSshBootstrapSeed, VmSshError, VmSshKeyMaterial, build_vm_ssh_client_args,
    render_vm_ssh_config_host,
};
pub use transport::{CommandOutput, TransportError, VmControlClient, VmKeeperClient, VmSession};

//! Effective authority compilation, exploration profile, secret-free plan
//! rendering, and platform-qualified resource budgets (`add-vm-project-runtime`
//! tasks 3.1 and 3.5).

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::contract::{ErrorLayer, LaunchRecordV1, ResourceBudgets};
use super::qualification::HostPlatform;

pub const MAX_VM_VCPUS: u32 = 8;
pub const MIN_VM_MEMORY_MIB: u32 = 128;
pub const MAX_VM_MEMORY_MIB: u32 = 4096;
pub const MIN_WRITABLE_DISK_MIB: u32 = 64;
pub const MAX_WRITABLE_DISK_MIB: u32 = 8192;
pub const MAX_LOG_BYTES_CAP: u64 = 16 * 1024 * 1024;
pub const MAX_READINESS_TIMEOUT_SECS: u64 = 300;

/// Provenance tag attached to every compiled permission entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorityOrigin {
    ProfileExploration,
    UserGrant,
    RuntimeHardLimit,
    ProjectRequest,
}

impl AuthorityOrigin {
    pub const fn as_str(&self) -> &'static str {
        match self {
            AuthorityOrigin::ProfileExploration => "profile:exploration",
            AuthorityOrigin::UserGrant => "user:grant",
            AuthorityOrigin::RuntimeHardLimit => "runtime:hard-limit",
            AuthorityOrigin::ProjectRequest => "project:request",
        }
    }
}

/// Versioned first-use profile for preparation and setup only (zero runtime
/// egress and zero credentials).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplorationProfileV1 {
    pub version: String,
    pub allowed_registries: Vec<String>,
}

impl ExplorationProfileV1 {
    pub fn default_profile() -> Self {
        Self {
            version: "2026.11.1".into(),
            allowed_registries: vec![
                "cache.nixos.org".into(),
                "dl-cdn.alpinelinux.org".into(),
                "registry.npmjs.org".into(),
                "index.crates.io".into(),
            ],
        }
    }
}

/// Authority requested by a project's committed or imported definitions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectAuthorityRequest {
    pub preparation_egress: Vec<String>,
    pub setup_egress: Vec<String>,
    pub runtime_egress: Vec<String>,
    pub requested_credentials: Vec<String>,
    pub requested_host_mounts: Vec<String>,
    pub requested_daemon_sockets: Vec<String>,
    pub requested_ssh_agent: bool,
    pub requested_device_passthrough: Vec<String>,
    pub requested_guest_grants: Vec<String>,
}

/// User-owned grants bound to `(project_root, definition_sha256)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserAuthorityGrants {
    pub project_root: PathBuf,
    pub definition_sha256: String,
    pub preparation_egress: Option<Vec<String>>,
    pub setup_egress: Option<Vec<String>>,
    pub runtime_egress: Option<Vec<String>>,
    pub authorized_credentials: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveEgressGrant {
    pub host: String,
    pub origin: AuthorityOrigin,
}

/// Mechanism enforcing finite budgets for the VM helper process on a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostResourceEnforcement {
    CgroupV2Scope,
    GuestAndLogBoundsOnly,
    Unsupported,
}

impl HostResourceEnforcement {
    pub const fn as_str(&self) -> &'static str {
        match self {
            HostResourceEnforcement::CgroupV2Scope => "cgroup-v2-scope",
            HostResourceEnforcement::GuestAndLogBoundsOnly => "guest-and-log-bounds-only",
            HostResourceEnforcement::Unsupported => "unsupported",
        }
    }
}

/// Platform resource control summary keeping guest RAM separate from host cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostResourceQualification {
    pub host: HostPlatform,
    pub enforcement: HostResourceEnforcement,
    pub guest_ram_is_total_host_cap: bool,
    pub enforced_controls: Vec<&'static str>,
    pub unsupported_controls: Vec<&'static str>,
}

/// Compiled effective authority after intersecting requests, user grants,
/// exploration profile, and hard limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledVmAuthority {
    pub preparation_egress: Vec<EffectiveEgressGrant>,
    pub setup_egress: Vec<EffectiveEgressGrant>,
    pub runtime_egress: Vec<EffectiveEgressGrant>,
    pub authorized_credential_names: Vec<String>,
    pub guest_grants: Vec<String>,
    pub budgets: ResourceBudgets,
    pub resource_qualification: HostResourceQualification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityError {
    UnsupportedRequest {
        kind: &'static str,
        detail: String,
    },
    UnresolvedCredentials(Vec<String>),
    BudgetExceeded {
        field: &'static str,
        reason: String,
    },
    UnsupportedHostResourceLimit {
        host: HostPlatform,
        control: &'static str,
    },
}

impl AuthorityError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            AuthorityError::UnsupportedHostResourceLimit { .. } => ErrorLayer::Backend,
            _ => ErrorLayer::Config,
        }
    }
}

impl fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthorityError::UnsupportedRequest { kind, detail } => {
                write!(f, "unsupported private-VM request `{kind}`: {detail}")
            }
            AuthorityError::UnresolvedCredentials(names) => write!(
                f,
                "credential requests lack user authorization: {}",
                names.join(", ")
            ),
            AuthorityError::BudgetExceeded { field, reason } => {
                write!(f, "resource budget `{field}` refused: {reason}")
            }
            AuthorityError::UnsupportedHostResourceLimit { host, control } => write!(
                f,
                "required host resource control `{control}` is unavailable on {}",
                host.as_str()
            ),
        }
    }
}

impl std::error::Error for AuthorityError {}

/// Report which resource controls are actually enforced per platform without
/// conflating configured guest RAM with a host-wide memory cap.
pub fn qualify_host_resource_controls(host: HostPlatform) -> HostResourceQualification {
    match host {
        HostPlatform::MacosArm64 => HostResourceQualification {
            host,
            enforcement: HostResourceEnforcement::GuestAndLogBoundsOnly,
            guest_ram_is_total_host_cap: false,
            enforced_controls: vec![
                "guest-vcpu-count",
                "guest-ram-allocation",
                "writable-ext4-capacity",
                "helper-log-cap",
                "transport-frame-and-deadline-bounds",
            ],
            unsupported_controls: vec!["cgroup_v2_host_memory_cap"],
        },
        HostPlatform::LinuxArm64 | HostPlatform::LinuxX86_64 => HostResourceQualification {
            host,
            enforcement: HostResourceEnforcement::CgroupV2Scope,
            guest_ram_is_total_host_cap: false,
            enforced_controls: vec![
                "cgroup-v2-host-memory-cap",
                "guest-vcpu-count",
                "guest-ram-allocation",
                "writable-ext4-capacity",
                "helper-log-cap",
            ],
            unsupported_controls: vec![],
        },
        HostPlatform::MacosX86_64 | HostPlatform::Other => HostResourceQualification {
            host,
            enforcement: HostResourceEnforcement::Unsupported,
            guest_ram_is_total_host_cap: false,
            enforced_controls: vec![],
            unsupported_controls: vec!["cgroup_v2_host_memory_cap", "qualified_helper_budgets"],
        },
    }
}

/// Validate finite budgets against hard runtime ceilings and fail closed if a
/// required host memory cap is unavailable on `host`.
pub fn validate_resource_budgets(
    budgets: &ResourceBudgets,
    require_host_memory_cap: bool,
    host: HostPlatform,
) -> Result<HostResourceQualification, AuthorityError> {
    if !(1..=MAX_VM_VCPUS).contains(&budgets.vcpus) {
        return Err(AuthorityError::BudgetExceeded {
            field: "vcpus",
            reason: format!("{} outside 1..={MAX_VM_VCPUS}", budgets.vcpus),
        });
    }
    if !(MIN_VM_MEMORY_MIB..=MAX_VM_MEMORY_MIB).contains(&budgets.guest_memory_mib) {
        return Err(AuthorityError::BudgetExceeded {
            field: "guest_memory_mib",
            reason: format!(
                "{} outside {MIN_VM_MEMORY_MIB}..={MAX_VM_MEMORY_MIB}",
                budgets.guest_memory_mib
            ),
        });
    }
    if !(MIN_WRITABLE_DISK_MIB..=MAX_WRITABLE_DISK_MIB).contains(&budgets.writable_disk_mib) {
        return Err(AuthorityError::BudgetExceeded {
            field: "writable_disk_mib",
            reason: format!(
                "{} outside {MIN_WRITABLE_DISK_MIB}..={MAX_WRITABLE_DISK_MIB}",
                budgets.writable_disk_mib
            ),
        });
    }
    if budgets.log_bytes_cap == 0 || budgets.log_bytes_cap > MAX_LOG_BYTES_CAP {
        return Err(AuthorityError::BudgetExceeded {
            field: "log_bytes_cap",
            reason: format!("{} outside 1..={MAX_LOG_BYTES_CAP}", budgets.log_bytes_cap),
        });
    }
    if budgets.readiness_timeout_secs == 0
        || budgets.readiness_timeout_secs > MAX_READINESS_TIMEOUT_SECS
    {
        return Err(AuthorityError::BudgetExceeded {
            field: "readiness_timeout_secs",
            reason: format!(
                "{} outside 1..={MAX_READINESS_TIMEOUT_SECS}",
                budgets.readiness_timeout_secs
            ),
        });
    }

    let qual = qualify_host_resource_controls(host);
    if qual.enforcement == HostResourceEnforcement::Unsupported {
        return Err(AuthorityError::UnsupportedHostResourceLimit {
            host,
            control: "qualified_helper_budgets",
        });
    }
    if require_host_memory_cap && qual.enforcement != HostResourceEnforcement::CgroupV2Scope {
        return Err(AuthorityError::UnsupportedHostResourceLimit {
            host,
            control: "cgroup_v2_host_memory_cap",
        });
    }
    Ok(qual)
}

fn intersect_phase_egress(
    requested: &[String],
    granted: Option<&[String]>,
    exploration_fallback: Option<&ExplorationProfileV1>,
) -> Vec<EffectiveEgressGrant> {
    if let Some(allowed) = granted {
        let allowed_set: BTreeSet<String> =
            allowed.iter().map(|h| h.to_ascii_lowercase()).collect();
        let mut out = Vec::new();
        for req in requested {
            let norm = req.to_ascii_lowercase();
            if allowed_set.contains(&norm)
                && !out.iter().any(|g: &EffectiveEgressGrant| g.host == norm)
            {
                out.push(EffectiveEgressGrant {
                    host: norm,
                    origin: AuthorityOrigin::UserGrant,
                });
            }
        }
        return out;
    }
    if let Some(exp) = exploration_fallback {
        return exp
            .allowed_registries
            .iter()
            .map(|h| EffectiveEgressGrant {
                host: h.to_ascii_lowercase(),
                origin: AuthorityOrigin::ProfileExploration,
            })
            .collect();
    }
    Vec::new()
}

/// Compile project declarations intersected with user grants, exploration
/// profile, and runtime hard limits (refusing unsupported requests/secrets).
pub fn compile_effective_authority(
    req: &ProjectAuthorityRequest,
    grants: Option<&UserAuthorityGrants>,
    exploration: &ExplorationProfileV1,
    budgets: &ResourceBudgets,
    require_host_memory_cap: bool,
    host: HostPlatform,
) -> Result<CompiledVmAuthority, AuthorityError> {
    if let Some(m) = req.requested_host_mounts.first() {
        return Err(AuthorityError::UnsupportedRequest {
            kind: "host-mount",
            detail: m.clone(),
        });
    }
    if let Some(s) = req.requested_daemon_sockets.first() {
        return Err(AuthorityError::UnsupportedRequest {
            kind: "daemon-socket",
            detail: s.clone(),
        });
    }
    if req.requested_ssh_agent {
        return Err(AuthorityError::UnsupportedRequest {
            kind: "ssh-agent",
            detail: "forwarding operator SSH agent is disabled in private VM mode".into(),
        });
    }
    if let Some(d) = req.requested_device_passthrough.first() {
        return Err(AuthorityError::UnsupportedRequest {
            kind: "device-passthrough",
            detail: d.clone(),
        });
    }

    let resource_qualification = validate_resource_budgets(budgets, require_host_memory_cap, host)?;

    let authorized_set: BTreeSet<&str> = grants
        .map(|g| {
            g.authorized_credentials
                .iter()
                .map(String::as_str)
                .collect()
        })
        .unwrap_or_default();
    let missing_creds: Vec<String> = req
        .requested_credentials
        .iter()
        .filter(|name| !authorized_set.contains(name.as_str()))
        .cloned()
        .collect();
    if !missing_creds.is_empty() {
        return Err(AuthorityError::UnresolvedCredentials(missing_creds));
    }

    let preparation_egress = intersect_phase_egress(
        &req.preparation_egress,
        grants.and_then(|g| g.preparation_egress.as_deref()),
        Some(exploration),
    );
    let setup_egress = intersect_phase_egress(
        &req.setup_egress,
        grants.and_then(|g| g.setup_egress.as_deref()),
        Some(exploration),
    );
    // Exploration profile never grants outbound hosts to runtime.
    let runtime_egress = intersect_phase_egress(
        &req.runtime_egress,
        grants.and_then(|g| g.runtime_egress.as_deref()),
        None,
    );

    Ok(CompiledVmAuthority {
        preparation_egress,
        setup_egress,
        runtime_egress,
        authorized_credential_names: req.requested_credentials.clone(),
        guest_grants: req.requested_guest_grants.clone(),
        budgets: budgets.clone(),
        resource_qualification,
    })
}

impl CompiledVmAuthority {
    /// Render the combined launch and authority plan with origins, host disks,
    /// transport channels, and credential reference names (never values).
    pub fn render_plan(&self, launch: &LaunchRecordV1) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "vm-plan: target={} adapter={} phase={:?} generation={}",
            launch.target.as_str(),
            launch.adapter.as_str(),
            launch.phase,
            launch.instance_generation
        );
        for d in &launch.disks {
            let mode = if d.read_only { "RO" } else { "RW" };
            let _ = writeln!(out, "  disk: id={} mode={mode} path={}", d.id, d.path);
        }
        let _ = writeln!(
            out,
            "  channels: control={} keeper={} ssh={} nic={} tsi={}",
            launch.channels.control_socket,
            launch.channels.keeper_socket,
            launch.channels.ssh_socket,
            launch.channels.allow_nic,
            launch.channels.allow_tsi
        );
        for grant in &self.guest_grants {
            let _ = writeln!(out, "  guest-grant: {grant}");
        }
        for (phase_name, entries) in [
            ("preparation", &self.preparation_egress),
            ("setup", &self.setup_egress),
            ("runtime", &self.runtime_egress),
        ] {
            for e in entries {
                let _ = writeln!(
                    out,
                    "  egress[{phase_name}]: {} (origin: {})",
                    e.host,
                    e.origin.as_str()
                );
            }
        }
        for cred in &self.authorized_credential_names {
            let _ = writeln!(out, "  credential-ref: {cred}");
        }
        let _ = writeln!(
            out,
            "  resources: vcpus={} guest_memory_mib={} disk_mib={} log_cap={} enforcement={} guest_ram_is_total_host_cap={}",
            self.budgets.vcpus,
            self.budgets.guest_memory_mib,
            self.budgets.writable_disk_mib,
            self.budgets.log_bytes_cap,
            self.resource_qualification.enforcement.as_str(),
            self.resource_qualification.guest_ram_is_total_host_cap
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::contract::{
        CONTRACT_SCHEMA_V1, DiskAttachment, GuestPhase, GuestTarget, HostChannels, VmAdapter,
    };

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample_budgets() -> ResourceBudgets {
        ResourceBudgets {
            vcpus: 2,
            guest_memory_mib: 512,
            writable_disk_mib: 1024,
            log_bytes_cap: 1024 * 1024,
            readiness_timeout_secs: 60,
        }
    }

    fn sample_launch() -> LaunchRecordV1 {
        LaunchRecordV1 {
            schema: CONTRACT_SCHEMA_V1,
            instance_generation: 3,
            target: GuestTarget::LinuxArm64,
            adapter: VmAdapter::Libkrun,
            phase: GuestPhase::Preparation,
            boot_artifact_sha256: SHA_A.into(),
            required_capabilities: vec!["nono".into()],
            disks: vec![DiskAttachment {
                id: "runtime".into(),
                path: "/trusted/runtime.img".into(),
                read_only: true,
                sha256: Some(SHA_A.into()),
            }],
            channels: HostChannels {
                control_socket: "/inst/control.sock".into(),
                keeper_socket: "/inst/keeper.sock".into(),
                ssh_socket: "/inst/ssh.sock".into(),
                proxy_socket: Some("/inst/proxy.sock".into()),
                allow_nic: false,
                allow_tsi: false,
            },
            resources: sample_budgets(),
        }
    }

    #[test]
    fn first_open_uses_exploration_for_prep_setup_and_gives_runtime_nothing() {
        let req = ProjectAuthorityRequest {
            runtime_egress: vec!["evil.example".into()],
            requested_guest_grants: vec!["/workspace:rw".into()],
            ..Default::default()
        };
        let compiled = compile_effective_authority(
            &req,
            None,
            &ExplorationProfileV1::default_profile(),
            &sample_budgets(),
            false,
            HostPlatform::MacosArm64,
        )
        .unwrap();

        assert!(!compiled.preparation_egress.is_empty());
        assert!(
            compiled
                .preparation_egress
                .iter()
                .all(|g| g.origin == AuthorityOrigin::ProfileExploration)
        );
        assert_eq!(
            compiled.setup_egress.len(),
            compiled.preparation_egress.len()
        );
        assert!(compiled.runtime_egress.is_empty());
        assert!(compiled.authorized_credential_names.is_empty());

        let plan = compiled.render_plan(&sample_launch());
        assert!(plan.contains("profile:exploration"));
        assert!(plan.contains("guest_ram_is_total_host_cap=false"));
    }

    #[test]
    fn unsupported_requests_and_unauthorized_credentials_are_refused() {
        let mut req = ProjectAuthorityRequest::default();
        req.requested_host_mounts.push("/Users/me/.aws".into());
        assert!(matches!(
            compile_effective_authority(
                &req,
                None,
                &ExplorationProfileV1::default_profile(),
                &sample_budgets(),
                false,
                HostPlatform::MacosArm64,
            ),
            Err(AuthorityError::UnsupportedRequest {
                kind: "host-mount",
                ..
            })
        ));

        let mut cred_req = ProjectAuthorityRequest::default();
        cred_req
            .requested_credentials
            .push("ANTHROPIC_API_KEY".into());
        assert!(matches!(
            compile_effective_authority(
                &cred_req,
                None,
                &ExplorationProfileV1::default_profile(),
                &sample_budgets(),
                false,
                HostPlatform::MacosArm64,
            ),
            Err(AuthorityError::UnresolvedCredentials(_))
        ));

        let grants = UserAuthorityGrants {
            project_root: PathBuf::from("/work/proj"),
            definition_sha256: SHA_A.into(),
            preparation_egress: None,
            setup_egress: None,
            runtime_egress: Some(vec!["api.anthropic.com".into()]),
            authorized_credentials: vec!["ANTHROPIC_API_KEY".into()],
        };
        cred_req.runtime_egress.push("api.anthropic.com".into());
        let ok = compile_effective_authority(
            &cred_req,
            Some(&grants),
            &ExplorationProfileV1::default_profile(),
            &sample_budgets(),
            false,
            HostPlatform::MacosArm64,
        )
        .unwrap();
        let plan = ok.render_plan(&sample_launch());
        assert!(plan.contains("credential-ref: ANTHROPIC_API_KEY"));
        assert!(plan.contains("user:grant"));
    }

    #[test]
    fn budgets_and_platform_resource_controls_fail_closed() {
        let mut huge = sample_budgets();
        huge.guest_memory_mib = 8192;
        assert!(matches!(
            validate_resource_budgets(&huge, false, HostPlatform::MacosArm64),
            Err(AuthorityError::BudgetExceeded {
                field: "guest_memory_mib",
                ..
            })
        ));

        let err = validate_resource_budgets(&sample_budgets(), true, HostPlatform::MacosArm64)
            .unwrap_err();
        assert_eq!(err.layer(), ErrorLayer::Backend);
        assert!(matches!(
            err,
            AuthorityError::UnsupportedHostResourceLimit {
                control: "cgroup_v2_host_memory_cap",
                ..
            }
        ));
    }
}

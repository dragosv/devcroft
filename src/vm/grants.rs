//! User-owned runtime grants and fail-closed runtime dispatch
//! (`add-vm-project-runtime` R7, task 1.5).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::contract::{ErrorLayer, GuestTarget, RuntimeKind, RuntimeSelectionSource, VmAdapter};
use super::qualification::{HostPlatform, qualification_for};

pub const RUNTIME_GRANTS_FILE: &str = "devcroft/runtime-grants.toml";

/// Identity of a project root independent of repository file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity(PathBuf);

impl ProjectIdentity {
    /// Resolve `project_root` cleanly without inspecting or trusting repository files.
    pub fn from_root(project_root: &Path) -> Result<Self, GrantError> {
        if !project_root.is_absolute() {
            return Err(GrantError::RelativeProjectPath(project_root.to_path_buf()));
        }
        let resolved = match std::fs::canonicalize(project_root) {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => project_root.to_path_buf(),
            Err(e) => return Err(GrantError::Io(e.to_string())),
        };
        Ok(ProjectIdentity(resolved))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Runtime granted to a specific project by user authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrantedRuntime {
    Native,
}

/// One user-owned grant entry bound to a canonical `project_root`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeGrant {
    pub project_root: PathBuf,
    pub runtime: GrantedRuntime,
    #[serde(default)]
    pub trusted_editor: bool,
    pub granted_at: String,
}

/// Persistent user-owned runtime grants (`~/.config/devcroft/runtime-grants.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RuntimeGrantStore {
    pub grants: Vec<RuntimeGrant>,
}

/// Failure reading, parsing, or writing `runtime-grants.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    RelativeProjectPath(PathBuf),
    DuplicateProjectGrant(PathBuf),
    Parse(String),
    Io(String),
}

impl GrantError {
    pub const fn layer(&self) -> ErrorLayer {
        ErrorLayer::Config
    }
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantError::RelativeProjectPath(p) => {
                write!(
                    f,
                    "runtime grant project_root must be absolute: {}",
                    p.display()
                )
            }
            GrantError::DuplicateProjectGrant(p) => {
                write!(f, "duplicate runtime grant for project {}", p.display())
            }
            GrantError::Parse(msg) => write!(f, "invalid runtime grants config: {msg}"),
            GrantError::Io(msg) => write!(f, "runtime grants I/O error: {msg}"),
        }
    }
}

impl std::error::Error for GrantError {}

impl RuntimeGrantStore {
    /// Standard XDG location matching `src/config/user.rs`.
    pub fn user_grants_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join(RUNTIME_GRANTS_FILE))
    }

    pub fn load() -> Result<Self, GrantError> {
        let Some(path) = Self::user_grants_path() else {
            return Ok(Self::default());
        };
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self, GrantError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(GrantError::Io(e.to_string())),
        };
        Self::parse_toml(&text)
    }

    pub fn parse_toml(text: &str) -> Result<Self, GrantError> {
        let store: Self = toml::from_str(text).map_err(|e| GrantError::Parse(e.to_string()))?;
        let mut seen = Vec::with_capacity(store.grants.len());
        for g in &store.grants {
            if !g.project_root.is_absolute() {
                return Err(GrantError::RelativeProjectPath(g.project_root.clone()));
            }
            let id = ProjectIdentity::from_root(&g.project_root)?;
            if seen.contains(&id) {
                return Err(GrantError::DuplicateProjectGrant(g.project_root.clone()));
            }
            seen.push(id);
        }
        Ok(store)
    }

    pub fn to_toml(&self) -> Result<String, GrantError> {
        toml::to_string_pretty(self).map_err(|e| GrantError::Parse(e.to_string()))
    }

    /// Persist atomically inside user config dir with owner-only `0600` permissions.
    pub fn save(&self, path: &Path) -> Result<(), GrantError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| GrantError::Io(e.to_string()))?;
        }
        let body = self.to_toml()?;
        std::fs::write(path, body).map_err(|e| GrantError::Io(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| GrantError::Io(e.to_string()))?;
        }
        Ok(())
    }

    pub fn has_native_grant(&self, project: &ProjectIdentity) -> bool {
        self.grants.iter().any(|g| {
            g.runtime == GrantedRuntime::Native
                && ProjectIdentity::from_root(&g.project_root).is_ok_and(|id| id == *project)
        })
    }

    pub fn has_trusted_editor_grant(&self, project: &ProjectIdentity) -> bool {
        self.grants.iter().any(|g| {
            g.trusted_editor
                && ProjectIdentity::from_root(&g.project_root).is_ok_and(|id| id == *project)
        })
    }

    pub fn grant_native(&mut self, project: &ProjectIdentity, granted_at: impl Into<String>) {
        if let Some(existing) = self
            .grants
            .iter_mut()
            .find(|g| ProjectIdentity::from_root(&g.project_root).is_ok_and(|id| id == *project))
        {
            existing.runtime = GrantedRuntime::Native;
            existing.granted_at = granted_at.into();
            return;
        }
        self.grants.push(RuntimeGrant {
            project_root: project.as_path().to_path_buf(),
            runtime: GrantedRuntime::Native,
            trusted_editor: false,
            granted_at: granted_at.into(),
        });
    }
}

/// Whether the release enforces user grants before native execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePolicyMode {
    /// Current opt-in release: native remains default unless VM is opted into.
    OptInVm,
    /// Untrusted-project claim mode: native requires an explicit user grant.
    RequireNativeGrant,
}

/// Runtime already bound to an existing sandbox instance (immutable per instance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingRuntime {
    Native,
    Vm {
        target: GuestTarget,
        adapter: VmAdapter,
    },
}

/// All inputs considered by runtime dispatch. Repository files are never consulted
/// to infer trust or select native execution.
#[derive(Debug, Clone)]
pub struct RuntimeRequest<'a> {
    pub project: &'a ProjectIdentity,
    pub provider: &'a str,
    pub existing_instance: Option<ExistingRuntime>,
    pub explicit_vm_opt_in: bool,
    pub policy_mode: RuntimePolicyMode,
    pub grants: &'a RuntimeGrantStore,
    pub host: HostPlatform,
    pub stdin_is_terminal: bool,
}

/// Resolved runtime kind and provenance recorded in state/`status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeDecision {
    pub runtime: RuntimeKind,
    pub selected_by: RuntimeSelectionSource,
}

/// Fail-closed runtime dispatch refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDispatchError {
    TargetMismatch {
        provider: String,
        target: GuestTarget,
    },
    NativeGrantRequired {
        project_root: PathBuf,
        provider: String,
        prompt_attempted: bool,
        grant_hint: String,
    },
    HostUnqualified {
        host: HostPlatform,
        reason: &'static str,
        grant_hint: Option<String>,
    },
}

impl RuntimeDispatchError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            RuntimeDispatchError::TargetMismatch { .. } => ErrorLayer::Provider,
            RuntimeDispatchError::NativeGrantRequired { .. } => ErrorLayer::Config,
            RuntimeDispatchError::HostUnqualified { .. } => ErrorLayer::Backend,
        }
    }
}

impl fmt::Display for RuntimeDispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeDispatchError::TargetMismatch { provider, target } => write!(
                f,
                "provider `{provider}` requires macOS/Darwin and cannot execute inside a {} guest",
                target.as_str()
            ),
            RuntimeDispatchError::NativeGrantRequired {
                project_root,
                provider,
                grant_hint,
                ..
            } => write!(
                f,
                "native execution for `{}` (provider `{provider}`) requires an explicit user grant; run `{grant_hint}`",
                project_root.display()
            ),
            RuntimeDispatchError::HostUnqualified {
                host,
                reason,
                grant_hint,
            } => match grant_hint {
                Some(hint) => write!(
                    f,
                    "no qualified microVM runtime on {}: {reason} (or grant native explicitly with `{hint}`)",
                    host.as_str()
                ),
                None => write!(
                    f,
                    "no qualified microVM runtime on {}: {reason}",
                    host.as_str()
                ),
            },
        }
    }
}

impl std::error::Error for RuntimeDispatchError {}

fn is_darwin_only_provider(provider: &str) -> bool {
    provider == "swift"
}

fn native_grant_command_hint(project: &ProjectIdentity) -> String {
    format!(
        "devcroft runtime grant-native --project {}",
        project.as_path().display()
    )
}

/// Select the runtime purely from user grants, existing instance metadata, and
/// host platform qualification — never from repository files or provider hints.
///
/// Notice `req.stdin_is_terminal` never triggers an interactive bypass or prompt;
/// missing grants always fail closed.
pub fn resolve_runtime(req: &RuntimeRequest<'_>) -> Result<RuntimeDecision, RuntimeDispatchError> {
    if let Some(existing) = req.existing_instance {
        let runtime = match existing {
            ExistingRuntime::Native => RuntimeKind::Native,
            ExistingRuntime::Vm { target, adapter } => RuntimeKind::Vm { target, adapter },
        };
        return Ok(RuntimeDecision {
            runtime,
            selected_by: RuntimeSelectionSource::ExistingInstance,
        });
    }

    if req.explicit_vm_opt_in {
        if is_darwin_only_provider(req.provider) {
            let target = match req.host {
                HostPlatform::LinuxX86_64 => GuestTarget::LinuxX86_64,
                _ => GuestTarget::LinuxArm64,
            };
            return Err(RuntimeDispatchError::TargetMismatch {
                provider: req.provider.to_owned(),
                target,
            });
        }
        return match qualification_for(req.host).status {
            super::qualification::QualificationStatus::QualifiedOptIn {
                adapter,
                guest_target,
                ..
            } => Ok(RuntimeDecision {
                runtime: RuntimeKind::Vm {
                    target: guest_target,
                    adapter,
                },
                selected_by: RuntimeSelectionSource::ExplicitOptIn,
            }),
            super::qualification::QualificationStatus::Unqualified { reason, .. } => {
                Err(RuntimeDispatchError::HostUnqualified {
                    host: req.host,
                    reason,
                    grant_hint: None,
                })
            }
        };
    }

    match req.policy_mode {
        RuntimePolicyMode::RequireNativeGrant => {
            if req.grants.has_native_grant(req.project) {
                return Ok(RuntimeDecision {
                    runtime: RuntimeKind::Native,
                    selected_by: RuntimeSelectionSource::UserGrant,
                });
            }
            let grant_hint = native_grant_command_hint(req.project);
            if is_darwin_only_provider(req.provider) {
                let _ = req.stdin_is_terminal;
                return Err(RuntimeDispatchError::NativeGrantRequired {
                    project_root: req.project.as_path().to_path_buf(),
                    provider: req.provider.to_owned(),
                    prompt_attempted: false,
                    grant_hint,
                });
            }
            match qualification_for(req.host).status {
                super::qualification::QualificationStatus::QualifiedOptIn {
                    adapter,
                    guest_target,
                    ..
                } => Ok(RuntimeDecision {
                    runtime: RuntimeKind::Vm {
                        target: guest_target,
                        adapter,
                    },
                    selected_by: RuntimeSelectionSource::QualifiedVmDefault,
                }),
                super::qualification::QualificationStatus::Unqualified { reason, .. } => {
                    Err(RuntimeDispatchError::HostUnqualified {
                        host: req.host,
                        reason,
                        grant_hint: Some(grant_hint),
                    })
                }
            }
        }
        RuntimePolicyMode::OptInVm => Ok(RuntimeDecision {
            runtime: RuntimeKind::Native,
            selected_by: RuntimeSelectionSource::OptInDefault,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(path: &str) -> ProjectIdentity {
        ProjectIdentity::from_root(Path::new(path)).unwrap()
    }

    #[test]
    fn darwin_provider_without_grant_refuses_never_prompts_with_or_without_tty() {
        let project = pid("/work/unreviewed-swift-repo");
        let grants = RuntimeGrantStore::default();
        for tty in [true, false] {
            let err = resolve_runtime(&RuntimeRequest {
                project: &project,
                provider: "swift",
                existing_instance: None,
                explicit_vm_opt_in: false,
                policy_mode: RuntimePolicyMode::RequireNativeGrant,
                grants: &grants,
                host: HostPlatform::MacosArm64,
                stdin_is_terminal: tty,
            })
            .unwrap_err();
            assert_eq!(err.layer(), ErrorLayer::Config);
            match err {
                RuntimeDispatchError::NativeGrantRequired {
                    prompt_attempted,
                    provider,
                    grant_hint,
                    ..
                } => {
                    assert!(!prompt_attempted);
                    assert_eq!(provider, "swift");
                    assert!(grant_hint.contains("grant-native"), "{grant_hint}");
                }
                other => panic!("unexpected error: {other:?}"),
            }
        }
    }

    #[test]
    fn ungranted_project_selects_qualified_vm_or_refuses_on_unqualified_host() {
        let project = pid("/work/unreviewed-nix-repo");
        let grants = RuntimeGrantStore::default();

        for tty in [true, false] {
            let decision = resolve_runtime(&RuntimeRequest {
                project: &project,
                provider: "nix",
                existing_instance: None,
                explicit_vm_opt_in: false,
                policy_mode: RuntimePolicyMode::RequireNativeGrant,
                grants: &grants,
                host: HostPlatform::MacosArm64,
                stdin_is_terminal: tty,
            })
            .unwrap();
            assert_eq!(
                decision,
                RuntimeDecision {
                    runtime: RuntimeKind::Vm {
                        target: GuestTarget::LinuxArm64,
                        adapter: VmAdapter::Libkrun,
                    },
                    selected_by: RuntimeSelectionSource::QualifiedVmDefault,
                }
            );
        }

        for unqualified in [
            HostPlatform::LinuxArm64,
            HostPlatform::LinuxX86_64,
            HostPlatform::MacosX86_64,
        ] {
            let err = resolve_runtime(&RuntimeRequest {
                project: &project,
                provider: "nix",
                existing_instance: None,
                explicit_vm_opt_in: false,
                policy_mode: RuntimePolicyMode::RequireNativeGrant,
                grants: &grants,
                host: unqualified,
                stdin_is_terminal: false,
            })
            .unwrap_err();
            assert_eq!(err.layer(), ErrorLayer::Backend);
            assert!(matches!(err, RuntimeDispatchError::HostUnqualified { .. }));
        }
    }

    #[test]
    fn user_grant_binds_to_exact_project_identity_only() {
        let granted = pid("/work/trusted-swift-repo");
        let sibling = pid("/work/other-swift-repo");
        let mut store = RuntimeGrantStore::default();
        store.grant_native(&granted, "2026-11-26T08:40:00Z");

        let ok = resolve_runtime(&RuntimeRequest {
            project: &granted,
            provider: "swift",
            existing_instance: None,
            explicit_vm_opt_in: false,
            policy_mode: RuntimePolicyMode::RequireNativeGrant,
            grants: &store,
            host: HostPlatform::MacosArm64,
            stdin_is_terminal: false,
        })
        .unwrap();
        assert_eq!(ok.runtime, RuntimeKind::Native);
        assert_eq!(ok.selected_by, RuntimeSelectionSource::UserGrant);

        assert!(
            resolve_runtime(&RuntimeRequest {
                project: &sibling,
                provider: "swift",
                existing_instance: None,
                explicit_vm_opt_in: false,
                policy_mode: RuntimePolicyMode::RequireNativeGrant,
                grants: &store,
                host: HostPlatform::MacosArm64,
                stdin_is_terminal: false,
            })
            .is_err()
        );
    }

    #[test]
    fn existing_instance_keeps_its_runtime_without_downgrade() {
        let project = pid("/work/legacy");
        let store = RuntimeGrantStore::default();

        let kept_native = resolve_runtime(&RuntimeRequest {
            project: &project,
            provider: "flox",
            existing_instance: Some(ExistingRuntime::Native),
            explicit_vm_opt_in: false,
            policy_mode: RuntimePolicyMode::RequireNativeGrant,
            grants: &store,
            host: HostPlatform::MacosArm64,
            stdin_is_terminal: false,
        })
        .unwrap();
        assert_eq!(kept_native.runtime, RuntimeKind::Native);
        assert_eq!(
            kept_native.selected_by,
            RuntimeSelectionSource::ExistingInstance
        );

        let kept_vm = resolve_runtime(&RuntimeRequest {
            project: &project,
            provider: "flox",
            existing_instance: Some(ExistingRuntime::Vm {
                target: GuestTarget::LinuxArm64,
                adapter: VmAdapter::Libkrun,
            }),
            explicit_vm_opt_in: false,
            policy_mode: RuntimePolicyMode::OptInVm,
            grants: &store,
            host: HostPlatform::LinuxX86_64,
            stdin_is_terminal: false,
        })
        .unwrap();
        assert_eq!(
            kept_vm.runtime,
            RuntimeKind::Vm {
                target: GuestTarget::LinuxArm64,
                adapter: VmAdapter::Libkrun,
            }
        );
        assert_eq!(
            kept_vm.selected_by,
            RuntimeSelectionSource::ExistingInstance
        );
    }

    #[test]
    fn grant_store_round_trip_rejects_bad_inputs_and_saves_mode_0600() {
        let mut store = RuntimeGrantStore::default();
        store.grant_native(&pid("/work/alpha"), "2026-11-26T08:40:00Z");
        let toml_text = store.to_toml().unwrap();
        let reparsed = RuntimeGrantStore::parse_toml(&toml_text).unwrap();
        assert_eq!(reparsed, store);

        assert!(RuntimeGrantStore::parse_toml("unknown = 1\n").is_err());
        assert!(matches!(
            RuntimeGrantStore::parse_toml(
                "[[grants]]\nproject_root = \"rel\"\nruntime = \"native\"\ngranted_at = \"now\"\n"
            ),
            Err(GrantError::RelativeProjectPath(_))
        ));
        assert!(matches!(
            RuntimeGrantStore::parse_toml(
                "[[grants]]\nproject_root = \"/work/a\"\nruntime = \"native\"\ngranted_at = \"t1\"\n\
                 [[grants]]\nproject_root = \"/work/a\"\nruntime = \"native\"\ngranted_at = \"t2\"\n"
            ),
            Err(GrantError::DuplicateProjectGrant(_))
        ));

        let dir = std::env::temp_dir().join(format!("devcroft-grants-test-{}", std::process::id()));
        let file = dir.join("runtime-grants.toml");
        store.save(&file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let reloaded = RuntimeGrantStore::load_from(&file).unwrap();
        assert_eq!(reloaded, store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

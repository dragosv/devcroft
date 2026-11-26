//! Private preparation job specs, per-provider Linux target requalification
//! (data-only bounded lockfile checks), declared input-set hashing, and Linux
//! profile validation (`add-vm-project-runtime` task 2.5).

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::contract::{CONTRACT_SCHEMA_V1, ErrorLayer, GuestTarget};
use super::sha256::{Sha256Hasher, sha256_file_hex};

/// Canonical guest project root used across both preparation and runtime.
pub const CANONICAL_GUEST_WORKSPACE: &str = "/workspace";

const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;
const FORBIDDEN_HOST_PREFIXES: &[&str] =
    &["/Users/", "/opt/homebrew/", "/Applications/", "/Library/"];

/// Input scope forwarded into an isolated preparation VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSetPolicy {
    DeclaredFiles(Vec<&'static str>),
    WholeSelectedSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashedInputFile {
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// Content identity computed strictly over the allowed input files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeIdentity {
    pub provider: String,
    pub target: GuestTarget,
    pub recipe_sha256: String,
    pub input_files: Vec<HashedInputFile>,
    pub whole_source_fallback: bool,
}

/// Isolated preparation VM job contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationJobSpecV1 {
    pub schema: u32,
    pub target: GuestTarget,
    pub provider: String,
    pub guest_workspace_path: String,
    pub recipe: RecipeIdentity,
    pub private_writable_store: bool,
    pub allow_shared_writable_store: bool,
    pub personal_credentials_injected: bool,
}

/// Environment/profile captured inside the Linux preparation VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedLinuxProfileV1 {
    pub schema: u32,
    pub target: GuestTarget,
    pub guest_workspace_path: String,
    pub env: BTreeMap<String, String>,
    pub unset: Vec<String>,
    pub hook_data: Option<String>,
    pub declared_services: Vec<String>,
    pub closure_roots: Vec<String>,
    pub generated_profile_sh: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparationError {
    DarwinOnlyProvider(String),
    UnsupportedProvider(String),
    UnlockedDefinition {
        provider: String,
        required_lock: &'static str,
    },
    LockMissingTarget {
        provider: String,
        target_system: &'static str,
    },
    InvalidLockfile {
        provider: String,
        reason: String,
    },
    UnsafeInputSymlink(String),
    HostLeakInProfile(String),
    Io(String),
}

impl PreparationError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            PreparationError::UnsafeInputSymlink(_) | PreparationError::HostLeakInProfile(_) => {
                ErrorLayer::Config
            }
            _ => ErrorLayer::Provider,
        }
    }
}

impl fmt::Display for PreparationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreparationError::DarwinOnlyProvider(p) => write!(
                f,
                "provider `{p}` targets macOS/Darwin and cannot be prepared in a Linux guest"
            ),
            PreparationError::UnsupportedProvider(p) => {
                write!(
                    f,
                    "provider `{p}` is not supported for Linux VM preparation"
                )
            }
            PreparationError::UnlockedDefinition {
                provider,
                required_lock,
            } => write!(
                f,
                "provider `{provider}` requires `{required_lock}` (preparation holds no credentials and never silently re-locks)"
            ),
            PreparationError::LockMissingTarget {
                provider,
                target_system,
            } => write!(
                f,
                "provider `{provider}` lockfile does not cover Linux target `{target_system}`"
            ),
            PreparationError::InvalidLockfile { provider, reason } => {
                write!(f, "invalid lockfile for provider `{provider}`: {reason}")
            }
            PreparationError::UnsafeInputSymlink(path) => {
                write!(f, "refusing symlink escaping project root: `{path}`")
            }
            PreparationError::HostLeakInProfile(msg) => {
                write!(
                    f,
                    "captured Linux profile contains invalid host path: {msg}"
                )
            }
            PreparationError::Io(msg) => write!(f, "preparation I/O error: {msg}"),
        }
    }
}

impl std::error::Error for PreparationError {}

fn linux_system_string(target: GuestTarget) -> &'static str {
    match target {
        GuestTarget::LinuxArm64 => "aarch64-linux",
        GuestTarget::LinuxX86_64 => "x86_64-linux",
    }
}

fn read_bounded_text(path: &Path, provider: &str) -> Result<String, PreparationError> {
    let mut file = File::open(path).map_err(|e| PreparationError::Io(e.to_string()))?;
    let mut buf = Vec::new();
    file.by_ref()
        .take(MAX_LOCKFILE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| PreparationError::Io(e.to_string()))?;
    if buf.len() as u64 > MAX_LOCKFILE_BYTES {
        return Err(PreparationError::InvalidLockfile {
            provider: provider.to_owned(),
            reason: format!("lockfile exceeds {MAX_LOCKFILE_BYTES} bytes"),
        });
    }
    String::from_utf8(buf).map_err(|e| PreparationError::InvalidLockfile {
        provider: provider.to_owned(),
        reason: e.to_string(),
    })
}

/// Perform bounded data-only validation that `provider`'s locked definition
/// covers `target` on Linux without executing project code.
pub fn requalify_provider_for_linux(
    provider: &str,
    project_root: &Path,
    target: GuestTarget,
) -> Result<InputSetPolicy, PreparationError> {
    let target_sys = linux_system_string(target);
    match provider {
        "swift" => Err(PreparationError::DarwinOnlyProvider(provider.to_owned())),
        "flox" => {
            let manifest = project_root.join(".flox/env/manifest.toml");
            let lock = project_root.join(".flox/env/manifest.lock");
            if !manifest.is_file() || !lock.is_file() {
                return Err(PreparationError::UnlockedDefinition {
                    provider: provider.into(),
                    required_lock: ".flox/env/manifest.lock",
                });
            }
            let text = read_bounded_text(&lock, provider)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| PreparationError::InvalidLockfile {
                    provider: provider.into(),
                    reason: e.to_string(),
                })?;
            let rendered = parsed.to_string();
            if !rendered.contains(target_sys) {
                return Err(PreparationError::LockMissingTarget {
                    provider: provider.into(),
                    target_system: target_sys,
                });
            }
            Ok(InputSetPolicy::DeclaredFiles(vec![
                ".flox/env/manifest.toml",
                ".flox/env/manifest.lock",
            ]))
        }
        "devbox" => {
            let cfg = project_root.join("devbox.json");
            let lock = project_root.join("devbox.lock");
            if !cfg.is_file() || !lock.is_file() {
                return Err(PreparationError::UnlockedDefinition {
                    provider: provider.into(),
                    required_lock: "devbox.lock",
                });
            }
            let text = read_bounded_text(&lock, provider)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| PreparationError::InvalidLockfile {
                    provider: provider.into(),
                    reason: e.to_string(),
                })?;
            let rendered = parsed.to_string();
            let mentions_darwin =
                rendered.contains("aarch64-darwin") || rendered.contains("x86_64-darwin");
            let mentions_linux = rendered.contains(target_sys);
            if mentions_darwin && !mentions_linux {
                return Err(PreparationError::LockMissingTarget {
                    provider: provider.into(),
                    target_system: target_sys,
                });
            }
            Ok(InputSetPolicy::DeclaredFiles(vec![
                "devbox.json",
                "devbox.lock",
            ]))
        }
        "nix" => {
            if !project_root.join("flake.nix").is_file()
                || !project_root.join("flake.lock").is_file()
            {
                return Err(PreparationError::UnlockedDefinition {
                    provider: provider.into(),
                    required_lock: "flake.lock",
                });
            }
            Ok(InputSetPolicy::WholeSelectedSource)
        }
        "devenv" => {
            if !project_root.join("devenv.nix").is_file()
                || !project_root.join("devenv.lock").is_file()
            {
                return Err(PreparationError::UnlockedDefinition {
                    provider: provider.into(),
                    required_lock: "devenv.lock",
                });
            }
            Ok(InputSetPolicy::WholeSelectedSource)
        }
        other => Err(PreparationError::UnsupportedProvider(other.to_owned())),
    }
}

fn collect_recursive_paths(
    canonical_root: &Path,
    dir: &Path,
    out: &mut Vec<HashedInputFile>,
) -> Result<(), PreparationError> {
    let entries = fs::read_dir(dir).map_err(|e| PreparationError::Io(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| PreparationError::Io(e.to_string()))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".git" || name_str == ".devcroft" {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| PreparationError::Io(e.to_string()))?;
        if meta.file_type().is_symlink() {
            let resolved = fs::canonicalize(&path)
                .map_err(|_| PreparationError::UnsafeInputSymlink(path.display().to_string()))?;
            if !resolved.starts_with(canonical_root) {
                return Err(PreparationError::UnsafeInputSymlink(
                    path.display().to_string(),
                ));
            }
            continue;
        }
        if meta.is_dir() {
            collect_recursive_paths(canonical_root, &path, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(canonical_root)
                .map_err(|e| PreparationError::Io(e.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let sha256 = sha256_file_hex(&path).map_err(|e| PreparationError::Io(e.to_string()))?;
            out.push(HashedInputFile {
                relative_path: rel,
                sha256,
                size_bytes: meta.len(),
            });
        }
    }
    Ok(())
}

/// Hash the declared input set (or entire selected source tree when a provider
/// cannot enumerate its imports) into a deterministic `RecipeIdentity`.
pub fn collect_recipe_identity(
    provider: &str,
    project_root: &Path,
    target: GuestTarget,
) -> Result<RecipeIdentity, PreparationError> {
    let policy = requalify_provider_for_linux(provider, project_root, target)?;
    let canonical_root =
        fs::canonicalize(project_root).map_err(|e| PreparationError::Io(e.to_string()))?;
    let mut input_files = Vec::new();
    let whole_source_fallback = match policy {
        InputSetPolicy::DeclaredFiles(files) => {
            for rel in files {
                let abs = canonical_root.join(rel);
                let meta =
                    fs::symlink_metadata(&abs).map_err(|e| PreparationError::Io(e.to_string()))?;
                if meta.file_type().is_symlink() {
                    let resolved = fs::canonicalize(&abs)
                        .map_err(|_| PreparationError::UnsafeInputSymlink(rel.to_owned()))?;
                    if !resolved.starts_with(&canonical_root) {
                        return Err(PreparationError::UnsafeInputSymlink(rel.to_owned()));
                    }
                }
                let sha256 =
                    sha256_file_hex(&abs).map_err(|e| PreparationError::Io(e.to_string()))?;
                let size_bytes = fs::metadata(&abs)
                    .map_err(|e| PreparationError::Io(e.to_string()))?
                    .len();
                input_files.push(HashedInputFile {
                    relative_path: rel.to_owned(),
                    sha256,
                    size_bytes,
                });
            }
            false
        }
        InputSetPolicy::WholeSelectedSource => {
            collect_recursive_paths(&canonical_root, &canonical_root, &mut input_files)?;
            true
        }
    };

    input_files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut hasher = Sha256Hasher::new();
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(target.as_str().as_bytes());
    hasher.update(b"\0");
    for f in &input_files {
        hasher.update(f.relative_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(f.sha256.as_bytes());
        hasher.update(b"\n");
    }

    Ok(RecipeIdentity {
        provider: provider.to_owned(),
        target,
        recipe_sha256: hasher.finalize_hex(),
        input_files,
        whole_source_fallback,
    })
}

impl PreparationJobSpecV1 {
    pub fn validate(&self) -> Result<(), PreparationError> {
        if self.schema != CONTRACT_SCHEMA_V1 {
            return Err(PreparationError::HostLeakInProfile(format!(
                "unsupported preparation schema {}",
                self.schema
            )));
        }
        if self.guest_workspace_path != CANONICAL_GUEST_WORKSPACE {
            return Err(PreparationError::HostLeakInProfile(format!(
                "guest_workspace_path `{}` must equal `{CANONICAL_GUEST_WORKSPACE}`",
                self.guest_workspace_path
            )));
        }
        if !self.private_writable_store
            || self.allow_shared_writable_store
            || self.personal_credentials_injected
        {
            return Err(PreparationError::HostLeakInProfile(
                "preparation requires a private writable store and zero personal credentials"
                    .into(),
            ));
        }
        Ok(())
    }
}

impl CapturedLinuxProfileV1 {
    /// Reject host-leaked macOS paths and enforce canonical `/workspace` binding.
    pub fn validate(&self) -> Result<(), PreparationError> {
        if self.schema != CONTRACT_SCHEMA_V1 {
            return Err(PreparationError::HostLeakInProfile(format!(
                "unsupported profile schema {}",
                self.schema
            )));
        }
        if self.guest_workspace_path != CANONICAL_GUEST_WORKSPACE {
            return Err(PreparationError::HostLeakInProfile(format!(
                "guest_workspace_path `{}` must equal `{CANONICAL_GUEST_WORKSPACE}`",
                self.guest_workspace_path
            )));
        }
        for root in &self.closure_roots {
            if let Some(prefix) = FORBIDDEN_HOST_PREFIXES
                .iter()
                .find(|p| root.starts_with(*p))
            {
                return Err(PreparationError::HostLeakInProfile(format!(
                    "closure_root `{root}` references host prefix `{prefix}`"
                )));
            }
        }
        if let Some(path_val) = self.env.get("PATH") {
            for seg in path_val.split(':') {
                if let Some(prefix) = FORBIDDEN_HOST_PREFIXES.iter().find(|p| seg.starts_with(*p)) {
                    return Err(PreparationError::HostLeakInProfile(format!(
                        "PATH entry `{seg}` references host prefix `{prefix}`"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mktemp(suffix: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("dc-vm-prep-{}-{suffix}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn swift_and_unlocked_or_darwin_only_locks_fail_before_execution() {
        let dir = mktemp("locks");
        assert!(matches!(
            requalify_provider_for_linux("swift", &dir, GuestTarget::LinuxArm64),
            Err(PreparationError::DarwinOnlyProvider(_))
        ));
        assert!(matches!(
            requalify_provider_for_linux("flox", &dir, GuestTarget::LinuxArm64),
            Err(PreparationError::UnlockedDefinition { .. })
        ));

        fs::create_dir_all(dir.join(".flox/env")).unwrap();
        fs::write(dir.join(".flox/env/manifest.toml"), "version = 1\n").unwrap();
        fs::write(
            dir.join(".flox/env/manifest.lock"),
            r#"{"packages":[{"system":"aarch64-darwin"}]}"#,
        )
        .unwrap();
        assert!(matches!(
            requalify_provider_for_linux("flox", &dir, GuestTarget::LinuxArm64),
            Err(PreparationError::LockMissingTarget {
                target_system: "aarch64-linux",
                ..
            })
        ));

        fs::write(dir.join("devbox.json"), "{}\n").unwrap();
        fs::write(
            dir.join("devbox.lock"),
            r#"{"packages":{"hello":{"systems":{"x86_64-darwin":{}}}}}"#,
        )
        .unwrap();
        assert!(matches!(
            requalify_provider_for_linux("devbox", &dir, GuestTarget::LinuxArm64),
            Err(PreparationError::LockMissingTarget {
                target_system: "aarch64-linux",
                ..
            })
        ));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn flox_hashes_only_declared_inputs_while_nix_tracks_whole_source() {
        let flox_dir = mktemp("flox");
        fs::create_dir_all(flox_dir.join(".flox/env")).unwrap();
        fs::create_dir_all(flox_dir.join("src")).unwrap();
        fs::write(flox_dir.join(".flox/env/manifest.toml"), "version = 1\n").unwrap();
        fs::write(
            flox_dir.join(".flox/env/manifest.lock"),
            r#"{"packages":[{"system":"aarch64-linux"}]}"#,
        )
        .unwrap();
        fs::write(flox_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let id1 = collect_recipe_identity("flox", &flox_dir, GuestTarget::LinuxArm64).unwrap();
        assert!(!id1.whole_source_fallback);
        assert_eq!(id1.input_files.len(), 2);

        fs::write(
            flox_dir.join("src/main.rs"),
            "fn main() { println!(\"changed\"); }\n",
        )
        .unwrap();
        let id2 = collect_recipe_identity("flox", &flox_dir, GuestTarget::LinuxArm64).unwrap();
        assert_eq!(id1.recipe_sha256, id2.recipe_sha256);

        let nix_dir = mktemp("nix");
        fs::create_dir_all(nix_dir.join("nix")).unwrap();
        fs::write(nix_dir.join("flake.nix"), "{ outputs = _: {}; }\n").unwrap();
        fs::write(nix_dir.join("flake.lock"), "{\"version\": 7}\n").unwrap();
        fs::write(nix_dir.join("nix/shell.nix"), "# v1\n").unwrap();

        let n1 = collect_recipe_identity("nix", &nix_dir, GuestTarget::LinuxArm64).unwrap();
        assert!(n1.whole_source_fallback);
        fs::write(nix_dir.join("nix/shell.nix"), "# v2\n").unwrap();
        let n2 = collect_recipe_identity("nix", &nix_dir, GuestTarget::LinuxArm64).unwrap();
        assert_ne!(n1.recipe_sha256, n2.recipe_sha256);

        let _ = fs::remove_dir_all(&flox_dir);
        let _ = fs::remove_dir_all(&nix_dir);
    }

    #[test]
    fn captured_linux_profile_rejects_macos_host_paths() {
        let mut env = BTreeMap::new();
        env.insert("PATH".into(), "/nix/store/abc-env/bin:/usr/bin:/bin".into());
        let valid = CapturedLinuxProfileV1 {
            schema: CONTRACT_SCHEMA_V1,
            target: GuestTarget::LinuxArm64,
            guest_workspace_path: CANONICAL_GUEST_WORKSPACE.into(),
            env,
            unset: vec!["HOME".into()],
            hook_data: None,
            declared_services: vec![],
            closure_roots: vec!["/nix/store/abc-env".into()],
            generated_profile_sh: "export FOO=1\n".into(),
        };
        valid.validate().unwrap();

        let mut leaked = valid;
        leaked
            .closure_roots
            .push("/Users/dragos/.nix-profile".into());
        assert!(matches!(
            leaked.validate(),
            Err(PreparationError::HostLeakInProfile(_))
        ));
    }
}

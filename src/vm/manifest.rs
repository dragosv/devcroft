//! Trusted bootstrap artifact manifest, controller compatibility checks, and
//! packaged layout verification (`add-vm-project-runtime` task 1.3).

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::contract::{CONTRACT_SCHEMA_V1, ErrorLayer, GuestTarget, is_valid_hex};

/// One verified artifact distributed through the controller-trusted channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactEntry {
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub license_id: String,
    pub source_url: String,
}

/// Complete immutable artifact bundle for a release version and guest target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedArtifactsV1 {
    pub helper: ArtifactEntry,
    pub kernel: ArtifactEntry,
    pub initramfs: ArtifactEntry,
    pub runtime_image: ArtifactEntry,
    pub preparation_image: ArtifactEntry,
    pub license_inventory: ArtifactEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<ArtifactEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_seed: Option<ArtifactEntry>,
}

/// User-trusted distribution manifest governing VM helper and guest artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedArtifactManifestV1 {
    pub schema: u32,
    pub manifest_version: String,
    pub min_compatible_controller: String,
    pub max_compatible_controller: String,
    pub publisher_key_id: String,
    pub signature_ed25519: String,
    pub target: GuestTarget,
    pub artifacts: TrustedArtifactsV1,
}

/// Resolved local paths under a trusted distribution directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedLayout {
    pub version_dir: PathBuf,
    pub helper: PathBuf,
    pub kernel: PathBuf,
    pub initramfs: PathBuf,
    pub runtime_image: PathBuf,
    pub preparation_image: PathBuf,
    pub license_inventory: PathBuf,
    pub workload: Option<PathBuf>,
    pub data_seed: Option<PathBuf>,
}

/// Failure validating or resolving a trusted artifact manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    UnsupportedSchema {
        expected: u32,
        found: u32,
    },
    EmptyField(&'static str),
    InvalidDigest {
        artifact: &'static str,
    },
    InvalidSignature,
    InvalidRelativePath {
        artifact: &'static str,
        path: String,
    },
    InvalidVersion(String),
    IncompatibleController {
        controller: String,
        min: String,
        max: String,
    },
    UntrustedProjectBootstrapPath {
        resolved: PathBuf,
        project_root: PathBuf,
    },
    Io(String),
}

impl ManifestError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            ManifestError::IncompatibleController { .. }
            | ManifestError::UntrustedProjectBootstrapPath { .. }
            | ManifestError::Io(_) => ErrorLayer::Backend,
            _ => ErrorLayer::Config,
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::UnsupportedSchema { expected, found } => {
                write!(
                    f,
                    "unsupported manifest schema {found}; expected {expected}"
                )
            }
            ManifestError::EmptyField(field) => {
                write!(f, "manifest field `{field}` must not be empty")
            }
            ManifestError::InvalidDigest { artifact } => {
                write!(
                    f,
                    "artifact `{artifact}` must carry a 64-character lowercase hex SHA-256"
                )
            }
            ManifestError::InvalidSignature => {
                write!(
                    f,
                    "manifest signature_ed25519 must be 128 lowercase hex characters"
                )
            }
            ManifestError::InvalidRelativePath { artifact, path } => {
                write!(
                    f,
                    "artifact `{artifact}` path `{path}` must be a normalized relative path without `..`"
                )
            }
            ManifestError::InvalidVersion(ver) => {
                write!(
                    f,
                    "invalid semantic version `{ver}` (expected MAJOR.MINOR.PATCH)"
                )
            }
            ManifestError::IncompatibleController {
                controller,
                min,
                max,
            } => write!(
                f,
                "controller version {controller} is outside manifest compatibility range [{min}, {max}]"
            ),
            ManifestError::UntrustedProjectBootstrapPath {
                resolved,
                project_root,
            } => write!(
                f,
                "refusing bootstrap artifact path {} inside repository root {}: bootstrap authority must come from user-trusted distribution",
                resolved.display(),
                project_root.display()
            ),
            ManifestError::Io(msg) => write!(f, "manifest error: {msg}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl TrustedArtifactManifestV1 {
    /// Parse strictly from JSON (`#[serde(deny_unknown_fields)]`) and validate.
    pub fn from_json_str(text: &str) -> Result<Self, ManifestError> {
        let parsed: Self =
            serde_json::from_str(text).map_err(|e| ManifestError::Io(e.to_string()))?;
        parsed.validate()?;
        Ok(parsed)
    }

    /// Validate schema, signature format, semver bounds, and all six artifacts.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != CONTRACT_SCHEMA_V1 {
            return Err(ManifestError::UnsupportedSchema {
                expected: CONTRACT_SCHEMA_V1,
                found: self.schema,
            });
        }
        if !is_safe_rel_component(&self.manifest_version) {
            return Err(ManifestError::InvalidRelativePath {
                artifact: "manifest_version",
                path: self.manifest_version.clone(),
            });
        }
        if self.publisher_key_id.trim().is_empty() {
            return Err(ManifestError::EmptyField("publisher_key_id"));
        }
        if !is_valid_hex(&self.signature_ed25519, 128) {
            return Err(ManifestError::InvalidSignature);
        }
        let min = parse_semver(&self.min_compatible_controller)?;
        let max = parse_semver(&self.max_compatible_controller)?;
        if min > max {
            return Err(ManifestError::IncompatibleController {
                controller: self.min_compatible_controller.clone(),
                min: self.min_compatible_controller.clone(),
                max: self.max_compatible_controller.clone(),
            });
        }
        for (name, entry) in self.named_artifacts() {
            validate_artifact_entry(name, entry)?;
        }
        if self.artifacts.workload.is_some() != self.artifacts.data_seed.is_some() {
            return Err(ManifestError::Io(
                "workload and data_seed must be provided together".into(),
            ));
        }
        if let Some(entry) = &self.artifacts.workload {
            validate_artifact_entry("workload", entry)?;
        }
        if let Some(entry) = &self.artifacts.data_seed {
            validate_artifact_entry("data_seed", entry)?;
        }
        Ok(())
    }

    /// Verify `controller_version` falls within `[min, max]`.
    pub fn check_controller_compatible(
        &self,
        controller_version: &str,
    ) -> Result<(), ManifestError> {
        let current = parse_semver(controller_version)?;
        let min = parse_semver(&self.min_compatible_controller)?;
        let max = parse_semver(&self.max_compatible_controller)?;
        if current < min || current > max {
            return Err(ManifestError::IncompatibleController {
                controller: controller_version.to_owned(),
                min: self.min_compatible_controller.clone(),
                max: self.max_compatible_controller.clone(),
            });
        }
        Ok(())
    }

    /// Resolve a trusted artifact under `trusted_root / manifest_version`, refusing
    /// any resolution inside `project_root`.
    pub fn resolve_trusted_path(
        &self,
        trusted_root: &Path,
        project_root: &Path,
        artifact_name: &'static str,
        entry: &ArtifactEntry,
    ) -> Result<PathBuf, ManifestError> {
        self.validate()?;
        validate_artifact_entry(artifact_name, entry)?;
        let version_dir = trusted_root.join(&self.manifest_version);
        let resolved = version_dir.join(&entry.relative_path);
        if path_is_within(trusted_root, project_root) || path_is_within(&resolved, project_root) {
            return Err(ManifestError::UntrustedProjectBootstrapPath {
                resolved,
                project_root: project_root.to_path_buf(),
            });
        }
        Ok(resolved)
    }

    /// Build the canonical `PackagedLayout` under `trusted_root` for `project_root`.
    pub fn packaged_layout(
        &self,
        trusted_root: &Path,
        project_root: &Path,
    ) -> Result<PackagedLayout, ManifestError> {
        self.validate()?;
        let version_dir = trusted_root.join(&self.manifest_version);
        Ok(PackagedLayout {
            version_dir,
            helper: self.resolve_trusted_path(
                trusted_root,
                project_root,
                "helper",
                &self.artifacts.helper,
            )?,
            kernel: self.resolve_trusted_path(
                trusted_root,
                project_root,
                "kernel",
                &self.artifacts.kernel,
            )?,
            initramfs: self.resolve_trusted_path(
                trusted_root,
                project_root,
                "initramfs",
                &self.artifacts.initramfs,
            )?,
            runtime_image: self.resolve_trusted_path(
                trusted_root,
                project_root,
                "runtime_image",
                &self.artifacts.runtime_image,
            )?,
            preparation_image: self.resolve_trusted_path(
                trusted_root,
                project_root,
                "preparation_image",
                &self.artifacts.preparation_image,
            )?,
            license_inventory: self.resolve_trusted_path(
                trusted_root,
                project_root,
                "license_inventory",
                &self.artifacts.license_inventory,
            )?,
            workload: self
                .artifacts
                .workload
                .as_ref()
                .map(|entry| {
                    self.resolve_trusted_path(trusted_root, project_root, "workload", entry)
                })
                .transpose()?,
            data_seed: self
                .artifacts
                .data_seed
                .as_ref()
                .map(|entry| {
                    self.resolve_trusted_path(trusted_root, project_root, "data_seed", entry)
                })
                .transpose()?,
        })
    }

    fn named_artifacts(&self) -> [(&'static str, &ArtifactEntry); 6] {
        [
            ("helper", &self.artifacts.helper),
            ("kernel", &self.artifacts.kernel),
            ("initramfs", &self.artifacts.initramfs),
            ("runtime_image", &self.artifacts.runtime_image),
            ("preparation_image", &self.artifacts.preparation_image),
            ("license_inventory", &self.artifacts.license_inventory),
        ]
    }
}

fn validate_artifact_entry(name: &'static str, entry: &ArtifactEntry) -> Result<(), ManifestError> {
    if !is_safe_relative_path(&entry.relative_path) {
        return Err(ManifestError::InvalidRelativePath {
            artifact: name,
            path: entry.relative_path.clone(),
        });
    }
    if !is_valid_hex(&entry.sha256, 64) {
        return Err(ManifestError::InvalidDigest { artifact: name });
    }
    if entry.size_bytes == 0 {
        return Err(ManifestError::EmptyField("size_bytes"));
    }
    if entry.license_id.trim().is_empty() {
        return Err(ManifestError::EmptyField("license_id"));
    }
    if entry.source_url.trim().is_empty() {
        return Err(ManifestError::EmptyField("source_url"));
    }
    Ok(())
}

fn is_safe_rel_component(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains('\\') && s != "." && s != ".."
}

fn is_safe_relative_path(raw: &str) -> bool {
    if raw.is_empty() || raw.contains('\\') {
        return false;
    }
    let mut saw_component = false;
    for comp in Path::new(raw).components() {
        match comp {
            Component::Normal(_) => saw_component = true,
            _ => return false,
        }
    }
    saw_component
}

fn path_is_within(candidate: &Path, root: &Path) -> bool {
    let cand_norm = std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
    let root_norm = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    cand_norm.starts_with(&root_norm)
}

fn parse_semver(raw: &str) -> Result<(u64, u64, u64), ManifestError> {
    let mut parts = raw.split('.');
    let major = parts
        .next()
        .and_then(|p| p.parse::<u64>().ok())
        .ok_or_else(|| ManifestError::InvalidVersion(raw.to_owned()))?;
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u64>().ok())
        .ok_or_else(|| ManifestError::InvalidVersion(raw.to_owned()))?;
    let patch_part = parts
        .next()
        .ok_or_else(|| ManifestError::InvalidVersion(raw.to_owned()))?;
    if parts.next().is_some() {
        return Err(ManifestError::InvalidVersion(raw.to_owned()));
    }
    let patch_digits = patch_part.split('-').next().unwrap_or(patch_part);
    let patch = patch_digits
        .parse::<u64>()
        .map_err(|_| ManifestError::InvalidVersion(raw.to_owned()))?;
    Ok((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SIG: &str = "22222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222";

    fn entry(rel: &str, license: &str) -> ArtifactEntry {
        ArtifactEntry {
            relative_path: rel.into(),
            sha256: SHA.into(),
            size_bytes: 1024,
            license_id: license.into(),
            source_url: "https://releases.devcroft.invalid/src.tar.xz".into(),
        }
    }

    fn sample_manifest() -> TrustedArtifactManifestV1 {
        TrustedArtifactManifestV1 {
            schema: CONTRACT_SCHEMA_V1,
            manifest_version: "2026.11.1".into(),
            min_compatible_controller: "0.0.1".into(),
            max_compatible_controller: "0.0.9".into(),
            publisher_key_id: "release-key-1".into(),
            signature_ed25519: SIG.into(),
            target: GuestTarget::LinuxArm64,
            artifacts: TrustedArtifactsV1 {
                helper: entry("bin/devcroft-vm-helper", "Apache-2.0"),
                kernel: entry("boot/vmlinux-6.12.95", "GPL-2.0-only"),
                initramfs: entry("boot/initramfs.cpio.zst", "Apache-2.0"),
                runtime_image: entry("images/runtime-base.img", "MIT AND GPL-2.0-only"),
                preparation_image: entry("images/prep-base.img", "Apache-2.0"),
                license_inventory: entry("licenses/vm-artifact-licenses.json", "Apache-2.0"),
                workload: None,
                data_seed: None,
            },
        }
    }

    #[test]
    fn valid_manifest_resolves_packaged_layout_and_controller_range() {
        let m = sample_manifest();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let parsed = TrustedArtifactManifestV1::from_json_str(&json).unwrap();
        assert_eq!(parsed, m);

        parsed.check_controller_compatible("0.0.1").unwrap();
        parsed.check_controller_compatible("0.0.5").unwrap();
        assert!(matches!(
            parsed.check_controller_compatible("0.1.0"),
            Err(ManifestError::IncompatibleController { .. })
        ));

        let layout = parsed
            .packaged_layout(
                Path::new("/Users/op/.local/share/devcroft/vm-artifacts"),
                Path::new("/Users/op/work/untrusted-repo"),
            )
            .unwrap();
        assert_eq!(
            layout.kernel,
            PathBuf::from(
                "/Users/op/.local/share/devcroft/vm-artifacts/2026.11.1/boot/vmlinux-6.12.95"
            )
        );
    }

    #[test]
    fn manifest_rejects_traversal_bad_digest_and_repo_bootstrap_paths() {
        let mut traversal = sample_manifest();
        traversal.artifacts.kernel.relative_path = "../escape/vmlinux".into();
        assert!(matches!(
            traversal.validate(),
            Err(ManifestError::InvalidRelativePath {
                artifact: "kernel",
                ..
            })
        ));

        let mut abs = sample_manifest();
        abs.artifacts.helper.relative_path = "/tmp/helper".into();
        assert!(matches!(
            abs.validate(),
            Err(ManifestError::InvalidRelativePath {
                artifact: "helper",
                ..
            })
        ));

        let mut bad_sha = sample_manifest();
        bad_sha.artifacts.initramfs.sha256 = "not-hex".into();
        assert!(matches!(
            bad_sha.validate(),
            Err(ManifestError::InvalidDigest {
                artifact: "initramfs"
            })
        ));

        let valid = sample_manifest();
        let err = valid
            .packaged_layout(
                Path::new("/work/repo/.devcroft-bootstrap"),
                Path::new("/work/repo"),
            )
            .unwrap_err();
        assert!(matches!(
            err,
            ManifestError::UntrustedProjectBootstrapPath { .. }
        ));
    }

    #[test]
    fn workload_and_data_seed_require_pairing_and_validate() {
        let mut partial = sample_manifest();
        partial.artifacts.workload = Some(entry("images/toolchain.img", "MIT"));
        assert!(partial.validate().is_err());

        partial.artifacts.data_seed = Some(entry("images/data.seed.img", "Apache-2.0"));
        partial.validate().unwrap();
        let layout = partial
            .packaged_layout(
                Path::new("/Users/op/.local/share/devcroft/vm-artifacts"),
                Path::new("/Users/op/work/untrusted-repo"),
            )
            .unwrap();
        assert!(layout.workload.is_some() && layout.data_seed.is_some());
    }
}

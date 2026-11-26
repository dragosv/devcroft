//! Controller-owned atomic artifact publication, host-side content hashing,
//! per-project cache namespaces, and retention leases (`add-vm-project-runtime`
//! task 2.6).

use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::contract::{CONTRACT_SCHEMA_V1, ErrorLayer, is_valid_hex};
use super::grants::ProjectIdentity;
use super::sha256::{sha256_file_hex, sha256_hex};

const MAX_ENVELOPE_BYTES: usize = 65536;

/// Whether a published artifact is restricted to its owning project namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CrossProjectEligibility {
    ProjectPrivateOnly,
    FixedOutputVerified { signature_key_id: String },
}

/// Bounded guest-produced output envelope carrying format and byte length only
/// (guests never supply host paths or cache identities).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestOutputEnvelopeV1 {
    pub schema: u32,
    pub format: String,
    pub size_bytes: u64,
    pub guest_notes: Option<String>,
}

/// Controller-owned provenance metadata persisted alongside the immutable blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedArtifactRecordV1 {
    pub schema: u32,
    pub content_sha256: String,
    pub recipe_sha256: String,
    pub project_namespace: String,
    pub format: String,
    pub size_bytes: u64,
    pub published_at: String,
    pub eligibility: CrossProjectEligibility,
}

/// Active consumer retention lease protecting a published artifact from GC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionLeaseV1 {
    pub instance_id: String,
    pub instance_generation: u64,
    pub acquired_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPublication {
    pub tx_id: String,
    pub staging_dir: PathBuf,
    pub blob_path: PathBuf,
    pub content_sha256: String,
    pub format: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationError {
    InvalidEnvelope(String),
    InvalidIdentifier(String),
    SubstitutionRefused(String),
    SizeMismatch { expected: u64, actual: u64 },
    ArtifactNotFound(String),
    Io(String),
}

impl PublicationError {
    pub const fn layer(&self) -> ErrorLayer {
        match self {
            PublicationError::SubstitutionRefused(_)
            | PublicationError::InvalidEnvelope(_)
            | PublicationError::SizeMismatch { .. } => ErrorLayer::Provider,
            _ => ErrorLayer::Config,
        }
    }
}

impl fmt::Display for PublicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PublicationError::InvalidEnvelope(msg) => {
                write!(f, "invalid guest publication envelope: {msg}")
            }
            PublicationError::InvalidIdentifier(msg) => {
                write!(f, "invalid publication identifier: {msg}")
            }
            PublicationError::SubstitutionRefused(msg) => {
                write!(f, "refusing cache artifact substitution: {msg}")
            }
            PublicationError::SizeMismatch { expected, actual } => write!(
                f,
                "staged artifact size {actual} does not match declared {expected}"
            ),
            PublicationError::ArtifactNotFound(id) => {
                write!(f, "published artifact `{id}` not found")
            }
            PublicationError::Io(msg) => write!(f, "publication I/O error: {msg}"),
        }
    }
}

impl std::error::Error for PublicationError {}

/// Controller-owned artifact store separating staging, per-project namespaces,
/// and explicitly verified cross-project fixed outputs.
#[derive(Debug, Clone)]
pub struct ArtifactCacheStore {
    pub root: PathBuf,
    pub max_artifact_bytes: u64,
}

impl ArtifactCacheStore {
    pub fn new(root: impl Into<PathBuf>, max_artifact_bytes: u64) -> Self {
        Self {
            root: root.into(),
            max_artifact_bytes,
        }
    }

    pub fn project_namespace_id(project: &ProjectIdentity) -> String {
        sha256_hex(project.as_path().to_string_lossy().as_bytes())
    }

    fn staging_root(&self) -> PathBuf {
        self.root.join("staging")
    }

    fn project_artifacts_dir(&self, project: &ProjectIdentity) -> PathBuf {
        self.root
            .join("namespaces")
            .join(Self::project_namespace_id(project))
            .join("artifacts")
    }

    fn shared_verified_dir(&self) -> PathBuf {
        self.root.join("shared-verified")
    }

    /// Stage guest output bytes into a quarantine directory and compute the
    /// content digest host-side without publishing it into any consumer lookup.
    pub fn stage_guest_output(
        &self,
        tx_id: &str,
        envelope_json: &[u8],
        payload_source: &Path,
    ) -> Result<StagedPublication, PublicationError> {
        validate_safe_id(tx_id, "tx_id")?;
        if envelope_json.len() > MAX_ENVELOPE_BYTES {
            return Err(PublicationError::InvalidEnvelope(format!(
                "envelope exceeds {MAX_ENVELOPE_BYTES} bytes"
            )));
        }
        let env: GuestOutputEnvelopeV1 = serde_json::from_slice(envelope_json)
            .map_err(|e| PublicationError::InvalidEnvelope(e.to_string()))?;
        if env.schema != CONTRACT_SCHEMA_V1 {
            return Err(PublicationError::InvalidEnvelope(format!(
                "unsupported schema {}",
                env.schema
            )));
        }
        if env.format != "ext4-raw" && env.format != "tar-zst" {
            return Err(PublicationError::InvalidEnvelope(format!(
                "unsupported format `{}`",
                env.format
            )));
        }
        if env.size_bytes == 0 || env.size_bytes > self.max_artifact_bytes {
            return Err(PublicationError::InvalidEnvelope(format!(
                "size_bytes {} outside 1..={}",
                env.size_bytes, self.max_artifact_bytes
            )));
        }

        let staging_dir = self.staging_root().join(tx_id);
        let _ = fs::remove_dir_all(&staging_dir);
        fs::create_dir_all(&staging_dir).map_err(|e| PublicationError::Io(e.to_string()))?;
        let blob_path = staging_dir.join("artifact.bin");
        fs::copy(payload_source, &blob_path).map_err(|e| PublicationError::Io(e.to_string()))?;
        let actual_len = fs::metadata(&blob_path)
            .map_err(|e| PublicationError::Io(e.to_string()))?
            .len();
        if actual_len != env.size_bytes {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(PublicationError::SizeMismatch {
                expected: env.size_bytes,
                actual: actual_len,
            });
        }
        let content_sha256 =
            sha256_file_hex(&blob_path).map_err(|e| PublicationError::Io(e.to_string()))?;
        fs::set_permissions(&blob_path, fs::Permissions::from_mode(0o444))
            .map_err(|e| PublicationError::Io(e.to_string()))?;

        Ok(StagedPublication {
            tx_id: tx_id.to_owned(),
            staging_dir,
            blob_path,
            content_sha256,
            format: env.format,
            size_bytes: env.size_bytes,
        })
    }

    /// Atomically commit a staged artifact under controller-assigned provenance
    /// into its project namespace (or `shared-verified` if explicitly verified).
    pub fn commit_publication(
        &self,
        staged: StagedPublication,
        project: &ProjectIdentity,
        recipe_sha256: &str,
        eligibility: CrossProjectEligibility,
        published_at: &str,
    ) -> Result<PublishedArtifactRecordV1, PublicationError> {
        if !is_valid_hex(recipe_sha256, 64) {
            return Err(PublicationError::InvalidIdentifier(
                "recipe_sha256 must be 64 lowercase hex chars".into(),
            ));
        }
        let project_namespace = Self::project_namespace_id(project);
        let target_parent = match &eligibility {
            CrossProjectEligibility::ProjectPrivateOnly => self.project_artifacts_dir(project),
            CrossProjectEligibility::FixedOutputVerified { .. } => self.shared_verified_dir(),
        };
        fs::create_dir_all(&target_parent).map_err(|e| PublicationError::Io(e.to_string()))?;
        let dest = target_parent.join(&staged.content_sha256);

        let record = PublishedArtifactRecordV1 {
            schema: CONTRACT_SCHEMA_V1,
            content_sha256: staged.content_sha256.clone(),
            recipe_sha256: recipe_sha256.to_owned(),
            project_namespace,
            format: staged.format.clone(),
            size_bytes: staged.size_bytes,
            published_at: published_at.to_owned(),
            eligibility,
        };

        if dest.exists() {
            let existing_rec = read_provenance(&dest.join("provenance.json"))?;
            let existing_sha = sha256_file_hex(&dest.join("artifact.bin"))
                .map_err(|e| PublicationError::Io(e.to_string()))?;
            let _ = cleanup_tree(&staged.staging_dir);
            if existing_sha != record.content_sha256
                || existing_rec.recipe_sha256 != record.recipe_sha256
                || existing_rec.project_namespace != record.project_namespace
            {
                return Err(PublicationError::SubstitutionRefused(format!(
                    "artifact {} already committed under different provenance",
                    record.content_sha256
                )));
            }
            return Ok(existing_rec);
        }

        let prov_path = staged.staging_dir.join("provenance.json");
        let prov_bytes = serde_json::to_vec_pretty(&record)
            .map_err(|e| PublicationError::InvalidEnvelope(e.to_string()))?;
        {
            let mut f =
                File::create(&prov_path).map_err(|e| PublicationError::Io(e.to_string()))?;
            f.write_all(&prov_bytes)
                .map_err(|e| PublicationError::Io(e.to_string()))?;
            f.sync_data()
                .map_err(|e| PublicationError::Io(e.to_string()))?;
        }
        fs::set_permissions(&prov_path, fs::Permissions::from_mode(0o444))
            .map_err(|e| PublicationError::Io(e.to_string()))?;
        fs::create_dir_all(staged.staging_dir.join("leases"))
            .map_err(|e| PublicationError::Io(e.to_string()))?;

        fs::rename(&staged.staging_dir, &dest).map_err(|e| PublicationError::Io(e.to_string()))?;
        Ok(record)
    }

    /// Look up a committed artifact matching `recipe_sha256` visible to `project`.
    /// Never reads `staging/` or other projects' private namespaces.
    pub fn lookup_for_project(
        &self,
        project: &ProjectIdentity,
        recipe_sha256: &str,
    ) -> Result<Option<PublishedArtifactRecordV1>, PublicationError> {
        for dir in [
            self.project_artifacts_dir(project),
            self.shared_verified_dir(),
        ] {
            if !dir.is_dir() {
                continue;
            }
            let entries = fs::read_dir(&dir).map_err(|e| PublicationError::Io(e.to_string()))?;
            for entry in entries {
                let entry = entry.map_err(|e| PublicationError::Io(e.to_string()))?;
                let prov = entry.path().join("provenance.json");
                if !prov.is_file() {
                    continue;
                }
                let rec = read_provenance(&prov)?;
                if rec.recipe_sha256 == recipe_sha256 {
                    return Ok(Some(rec));
                }
            }
        }
        Ok(None)
    }

    fn locate_artifact_dir(
        &self,
        project: &ProjectIdentity,
        content_sha256: &str,
    ) -> Result<PathBuf, PublicationError> {
        if !is_valid_hex(content_sha256, 64) {
            return Err(PublicationError::InvalidIdentifier(
                "content_sha256 must be 64 lowercase hex chars".into(),
            ));
        }
        let priv_dir = self.project_artifacts_dir(project).join(content_sha256);
        if priv_dir.is_dir() {
            return Ok(priv_dir);
        }
        let shared_dir = self.shared_verified_dir().join(content_sha256);
        if shared_dir.is_dir() {
            return Ok(shared_dir);
        }
        Err(PublicationError::ArtifactNotFound(
            content_sha256.to_owned(),
        ))
    }

    pub fn acquire_lease(
        &self,
        project: &ProjectIdentity,
        content_sha256: &str,
        lease: &RetentionLeaseV1,
    ) -> Result<PathBuf, PublicationError> {
        validate_safe_id(&lease.instance_id, "instance_id")?;
        let artifact_dir = self.locate_artifact_dir(project, content_sha256)?;
        let leases_dir = artifact_dir.join("leases");
        fs::create_dir_all(&leases_dir).map_err(|e| PublicationError::Io(e.to_string()))?;
        let lease_file = leases_dir.join(format!("{}.json", lease.instance_id));
        let bytes = serde_json::to_vec_pretty(lease)
            .map_err(|e| PublicationError::InvalidEnvelope(e.to_string()))?;
        fs::write(&lease_file, bytes).map_err(|e| PublicationError::Io(e.to_string()))?;
        Ok(lease_file)
    }

    pub fn release_lease(
        &self,
        project: &ProjectIdentity,
        content_sha256: &str,
        instance_id: &str,
    ) -> Result<(), PublicationError> {
        validate_safe_id(instance_id, "instance_id")?;
        let artifact_dir = self.locate_artifact_dir(project, content_sha256)?;
        let lease_file = artifact_dir
            .join("leases")
            .join(format!("{instance_id}.json"));
        match fs::remove_file(&lease_file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PublicationError::Io(e.to_string())),
        }
    }

    /// Garbage-collect unleased artifacts within `project`'s namespace only,
    /// leaving any artifact with at least one active lease intact.
    pub fn collect_unleased(
        &self,
        project: &ProjectIdentity,
    ) -> Result<Vec<String>, PublicationError> {
        let dir = self.project_artifacts_dir(project);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut removed = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| PublicationError::Io(e.to_string()))? {
            let entry = entry.map_err(|e| PublicationError::Io(e.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let leases_dir = path.join("leases");
            let has_active_lease = leases_dir.is_dir()
                && fs::read_dir(&leases_dir)
                    .map_err(|e| PublicationError::Io(e.to_string()))?
                    .any(|e| e.is_ok());
            if !has_active_lease {
                let id = entry.file_name().to_string_lossy().into_owned();
                cleanup_tree(&path)?;
                removed.push(id);
            }
        }
        removed.sort();
        Ok(removed)
    }
}

fn validate_safe_id(s: &str, field: &'static str) -> Result<(), PublicationError> {
    if s.is_empty()
        || s.contains('/')
        || s.contains('\\')
        || s.contains("..")
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(PublicationError::InvalidIdentifier(format!(
            "`{field}` must be non-empty [A-Za-z0-9_-]"
        )));
    }
    Ok(())
}

fn read_provenance(path: &Path) -> Result<PublishedArtifactRecordV1, PublicationError> {
    let bytes = fs::read(path).map_err(|e| PublicationError::Io(e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|e| PublicationError::InvalidEnvelope(e.to_string()))
}

fn cleanup_tree(path: &Path) -> Result<(), PublicationError> {
    if !path.exists() {
        return Ok(());
    }
    for sub in ["artifact.bin", "provenance.json"] {
        let p = path.join(sub);
        if p.exists() {
            let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
        }
    }
    fs::remove_dir_all(path).map_err(|e| PublicationError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const RECIPE_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn interrupted_stage_stays_quarantined_and_namespaces_isolate_projects() {
        let base = std::env::temp_dir().join(format!("dc-vm-pub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let store = ArtifactCacheStore::new(base.join("cache"), 1024 * 1024);
        let payload = base.join("guest-out.img");
        let bytes = b"immutable-runtime-ext4-bytes";
        fs::write(&payload, bytes).unwrap();
        let envelope = format!(
            r#"{{"schema":1,"format":"ext4-raw","size_bytes":{},"guest_notes":null}}"#,
            bytes.len()
        );

        let proj_a = ProjectIdentity::from_root(Path::new("/work/project-a")).unwrap();
        let proj_b = ProjectIdentity::from_root(Path::new("/work/project-b")).unwrap();

        // Interrupted stage leaves nothing visible to consumer lookup.
        let staged = store
            .stage_guest_output("tx-1", envelope.as_bytes(), &payload)
            .unwrap();
        assert!(
            store
                .lookup_for_project(&proj_a, RECIPE_A)
                .unwrap()
                .is_none()
        );

        // Commit isolates Project A's artifact from Project B under identical recipe.
        let committed = store
            .commit_publication(
                staged,
                &proj_a,
                RECIPE_A,
                CrossProjectEligibility::ProjectPrivateOnly,
                "2026-11-26T09:05:00Z",
            )
            .unwrap();
        assert_eq!(committed.content_sha256, sha256_hex(bytes));
        assert!(
            store
                .lookup_for_project(&proj_a, RECIPE_A)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .lookup_for_project(&proj_b, RECIPE_A)
                .unwrap()
                .is_none()
        );

        // Substitution / host-path envelope attempts are rejected cleanly.
        let forged = br#"{"schema":1,"format":"ext4-raw","size_bytes":10,"guest_notes":null,"host_path":"/etc/passwd"}"#;
        assert!(matches!(
            store.stage_guest_output("tx-bad", forged, &payload),
            Err(PublicationError::InvalidEnvelope(_))
        ));

        let staged_dup = store
            .stage_guest_output("tx-2", envelope.as_bytes(), &payload)
            .unwrap();
        assert!(matches!(
            store.commit_publication(
                staged_dup,
                &proj_a,
                RECIPE_B,
                CrossProjectEligibility::ProjectPrivateOnly,
                "2026-11-26T09:06:00Z",
            ),
            Err(PublicationError::SubstitutionRefused(_))
        ));

        // Retention leases protect artifacts until the last consumer releases.
        store
            .acquire_lease(
                &proj_a,
                &committed.content_sha256,
                &RetentionLeaseV1 {
                    instance_id: "inst-1".into(),
                    instance_generation: 1,
                    acquired_at: "2026-11-26T09:06:01Z".into(),
                },
            )
            .unwrap();
        store
            .acquire_lease(
                &proj_a,
                &committed.content_sha256,
                &RetentionLeaseV1 {
                    instance_id: "inst-2".into(),
                    instance_generation: 1,
                    acquired_at: "2026-11-26T09:06:02Z".into(),
                },
            )
            .unwrap();
        assert!(store.collect_unleased(&proj_a).unwrap().is_empty());

        store
            .release_lease(&proj_a, &committed.content_sha256, "inst-1")
            .unwrap();
        assert!(store.collect_unleased(&proj_a).unwrap().is_empty());

        store
            .release_lease(&proj_a, &committed.content_sha256, "inst-2")
            .unwrap();
        let reclaimed = store.collect_unleased(&proj_a).unwrap();
        assert_eq!(reclaimed, vec![committed.content_sha256]);
        assert!(
            store
                .lookup_for_project(&proj_a, RECIPE_A)
                .unwrap()
                .is_none()
        );

        let _ = fs::remove_dir_all(&base);
    }
}

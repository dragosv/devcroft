//! The `flox` provider (task 3.2): MVP's only implemented environment
//! provider. Activation runs once, host-side, before any sandbox
//! restriction (design.md decision 2) — this module never runs inside the
//! boundary and never activates per session.

use super::{Provider, ProviderError, Resolution};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Nix/flox's default store root. Overridden per-resolution when the
/// activated `PATH` names a different one (see [`store_grants`]).
const DEFAULT_STORE_ROOT: &str = "/nix/store";

pub struct FloxProvider;

impl Provider for FloxProvider {
    /// Resolve a flox environment: verify `.flox/` exists, run `flox
    /// activate -- env -0` once to capture the post-activation environment,
    /// diff it against this process's own environment, and derive the
    /// read-only store grants the compiled policy must carry.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_environment_present(project_root)?;

        let baseline: BTreeMap<String, String> = std::env::vars().collect();
        let activated = capture_activated_env(project_root)?;

        Ok(Resolution {
            env: diff_env(&baseline, &activated),
            read_only_grants: store_grants(&activated),
        })
    }
}

/// `up` fails at layer `provider` with the `flox init` hint (spec: "Missing
/// environment, not missing feature") rather than letting `flox activate`
/// produce its own, less specific error.
fn ensure_environment_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join(".flox").is_dir() {
        Ok(())
    } else {
        Err(ProviderError::NoEnvironment)
    }
}

fn capture_activated_env(project_root: &Path) -> Result<BTreeMap<String, String>, ProviderError> {
    let output = Command::new("flox")
        .arg("activate")
        .arg("--")
        .arg("env")
        .arg("-0")
        .current_dir(project_root)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProviderError::MissingBinary
            } else {
                ProviderError::ResolutionFailed(format!("running `flox activate`: {e}"))
            }
        })?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`flox activate` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(parse_env_dump(&output.stdout))
}

/// Parse `env -0` output: NUL-separated `KEY=VALUE` entries.
fn parse_env_dump(raw: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8_lossy(raw)
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| entry.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Entries added or changed by activation. Removed entries are not carried
/// forward — the keeper's own baseline environment is authoritative for
/// anything flox did not touch.
fn diff_env(
    baseline: &BTreeMap<String, String>,
    activated: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    activated
        .iter()
        .filter(|(k, v)| baseline.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The read-only store root(s) the compiled policy must grant for the
/// activated toolchain to run (spec: "Store paths become readable"). Reads
/// the store root out of the activated `PATH` so a non-default flox store
/// location is still detected, falling back to the conventional path.
fn store_grants(activated: &BTreeMap<String, String>) -> Vec<String> {
    let path = activated.get("PATH").map(String::as_str).unwrap_or("");
    for entry in path.split(':') {
        if let Some(idx) = entry.find("/nix/store") {
            return vec![entry[..idx + "/nix/store".len()].to_string()];
        }
    }
    vec![DEFAULT_STORE_ROOT.to_string()]
}

/// Content fingerprint of a flox environment's `manifest.toml` + lockfile,
/// for staleness detection (spec: "Stale environment after manifest
/// change"). `status` (task 4.3) compares this against the fingerprint
/// recorded at the last `up` to decide whether to suggest `--recreate`.
pub fn manifest_fingerprint(project_root: &Path) -> Result<String, ProviderError> {
    ensure_environment_present(project_root)?;
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read(&manifest_path).map_err(|e| {
        ProviderError::ResolutionFailed(format!("reading {}: {e}", manifest_path.display()))
    })?;
    // The lockfile does not exist until the first activation; its absence
    // is itself part of what makes a fingerprint change once one appears.
    let lock = std::fs::read(project_root.join(".flox/env/manifest.lock")).unwrap_or_default();

    Ok(fingerprint(&[&manifest, &lock]))
}

/// Whether the environment has changed since `recorded` was captured.
pub fn is_stale(project_root: &Path, recorded: &str) -> Result<bool, ProviderError> {
    Ok(manifest_fingerprint(project_root)? != recorded)
}

/// FNV-1a 64-bit over `parts`, each separated so `("ab", "c")` and
/// `("a", "bc")` never collide. Not cryptographic — this is a change
/// detector, not a security boundary, so no external hashing crate is
/// pulled in for it.
fn fingerprint(parts: &[&[u8]]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for part in parts {
        for &byte in *part {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("devcroft-flox-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn flox_env(root: &Path, manifest: &str, lock: Option<&str>) {
        let env_dir = root.join(".flox/env");
        fs::create_dir_all(&env_dir).unwrap();
        fs::write(env_dir.join("manifest.toml"), manifest).unwrap();
        if let Some(lock) = lock {
            fs::write(env_dir.join("manifest.lock"), lock).unwrap();
        }
    }

    #[test]
    fn parse_env_dump_splits_nul_separated_pairs() {
        let raw = b"FOO=bar\0PATH=/a:/b\0EMPTY=\0";
        let parsed = parse_env_dump(raw);
        assert_eq!(parsed.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(parsed.get("PATH"), Some(&"/a:/b".to_string()));
        assert_eq!(parsed.get("EMPTY"), Some(&"".to_string()));
    }

    #[test]
    fn diff_env_keeps_only_added_or_changed_keys() {
        let baseline = BTreeMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("UNCHANGED".to_string(), "same".to_string()),
        ]);
        let activated = BTreeMap::from([
            (
                "PATH".to_string(),
                "/nix/store/xyz-flox/bin:/usr/bin".to_string(),
            ),
            ("UNCHANGED".to_string(), "same".to_string()),
            ("FLOX_ENV".to_string(), "/nix/store/xyz-flox".to_string()),
        ]);

        let diff = diff_env(&baseline, &activated);

        assert_eq!(diff.len(), 2);
        assert_eq!(
            diff.get("PATH"),
            Some(&"/nix/store/xyz-flox/bin:/usr/bin".to_string())
        );
        assert_eq!(
            diff.get("FLOX_ENV"),
            Some(&"/nix/store/xyz-flox".to_string())
        );
        assert!(!diff.contains_key("UNCHANGED"));
    }

    #[test]
    fn store_grants_reads_root_from_activated_path() {
        let activated = BTreeMap::from([(
            "PATH".to_string(),
            "/nix/store/xyz-flox/bin:/usr/bin".to_string(),
        )]);
        assert_eq!(store_grants(&activated), vec!["/nix/store".to_string()]);
    }

    #[test]
    fn store_grants_falls_back_to_default_when_path_has_no_store_entry() {
        let activated = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        assert_eq!(
            store_grants(&activated),
            vec![DEFAULT_STORE_ROOT.to_string()]
        );
    }

    #[test]
    fn resolve_fails_with_no_environment_when_flox_dir_missing() {
        let root = tempdir("no-env");
        let err = FloxProvider.resolve(&root).unwrap_err();
        assert_eq!(err, ProviderError::NoEnvironment);
    }

    #[test]
    fn manifest_fingerprint_changes_when_manifest_changes() {
        let root = tempdir("fingerprint-manifest");
        flox_env(&root, "version = 1\n", Some("locked-a"));
        let before = manifest_fingerprint(&root).unwrap();

        flox_env(&root, "version = 2\n", Some("locked-a"));
        let after = manifest_fingerprint(&root).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn manifest_fingerprint_is_stable_for_unchanged_content() {
        let root = tempdir("fingerprint-stable");
        flox_env(&root, "version = 1\n", Some("locked-a"));

        let first = manifest_fingerprint(&root).unwrap();
        let second = manifest_fingerprint(&root).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn is_stale_true_after_manifest_changes() {
        let root = tempdir("stale-check");
        flox_env(&root, "version = 1\n", Some("locked-a"));
        let recorded = manifest_fingerprint(&root).unwrap();

        assert!(!is_stale(&root, &recorded).unwrap());

        flox_env(&root, "version = 2\n", Some("locked-a"));
        assert!(is_stale(&root, &recorded).unwrap());
    }

    #[test]
    fn manifest_fingerprint_fails_without_flox_environment() {
        let root = tempdir("fingerprint-missing");
        let err = manifest_fingerprint(&root).unwrap_err();
        assert_eq!(err, ProviderError::NoEnvironment);
    }

    #[test]
    fn resolve_against_real_flox_environment_if_available() {
        // Best-effort integration check: only runs meaningfully where
        // `flox` is installed and a real environment can be initialized
        // (the devcontainer provides both — see task 3.2's Dockerfile
        // change). Skips quietly otherwise so the suite stays portable.
        if Command::new("flox").arg("--version").output().is_err() {
            return;
        }
        let root = tempdir("real-resolve");
        let init = Command::new("flox")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        if !init.status.success() {
            return;
        }

        let resolution = FloxProvider.resolve(&root).unwrap();
        assert!(resolution.env.contains_key("PATH"));
        assert!(!resolution.read_only_grants.is_empty());
    }
}

//! The `flox` provider (task 3.2): MVP's only implemented environment
//! provider until add-nix-provider. Activation runs once, host-side,
//! before any sandbox restriction (design.md decision 2) — this module
//! never runs inside the boundary and never activates per session. Shared
//! capture/diff/fingerprint machinery lives in `provider::capture`.

use super::capture;
use super::{Provider, ProviderError, Resolution};
use crate::paths::resolve_on_path;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub struct FloxProvider;

impl Provider for FloxProvider {
    /// Resolve a flox environment: verify `.flox/` exists, run `flox
    /// activate -- env -0` once to capture the post-activation environment,
    /// diff it against the same fixed base activation ran with, and derive
    /// the read-only store grants the compiled policy must carry.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_environment_present(project_root)?;

        let baseline = capture::canonical_base_env()?;
        let activated = capture_activated_env(project_root, &baseline)?;

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: capture::store_grants(&activated),
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
        Err(ProviderError::NoEnvironment {
            provider: "flox",
            hint: "flox init",
        })
    }
}

fn capture_activated_env(
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ProviderError> {
    let flox_bin = resolve_on_path("flox").ok_or(ProviderError::MissingBinary {
        provider: "flox",
        hint: "devcroft doctor",
    })?;

    let output = Command::new(flox_bin)
        .arg("activate")
        .arg("--")
        .arg("env")
        .arg("-0")
        .current_dir(project_root)
        .env_clear()
        .envs(base)
        .output()
        .map_err(|e| ProviderError::ResolutionFailed(format!("running `flox activate`: {e}")))?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`flox activate` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(capture::parse_env_dump(&output.stdout))
}

/// Content fingerprint of a flox environment's `manifest.toml` + lockfile,
/// for staleness detection (spec: "Stale environment after manifest
/// change"). `provider::is_stale` (dispatched by provider name) compares
/// this against the fingerprint recorded at the last `up`.
pub fn manifest_fingerprint(project_root: &Path) -> Result<String, ProviderError> {
    ensure_environment_present(project_root)?;
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read(&manifest_path).map_err(|e| {
        ProviderError::ResolutionFailed(format!("reading {}: {e}", manifest_path.display()))
    })?;
    // The lockfile does not exist until the first activation; its absence
    // is itself part of what makes a fingerprint change once one appears.
    let lock = std::fs::read(project_root.join(".flox/env/manifest.lock")).unwrap_or_default();

    Ok(capture::fingerprint(&[&manifest, &lock]))
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
    fn resolve_fails_with_no_environment_when_flox_dir_missing() {
        let root = tempdir("no-env");
        let err = FloxProvider.resolve(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "flox",
                hint: "flox init"
            }
        );
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
    fn manifest_fingerprint_fails_without_flox_environment() {
        let root = tempdir("fingerprint-missing");
        let err = manifest_fingerprint(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "flox",
                hint: "flox init"
            }
        );
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

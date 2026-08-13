//! The `flox` provider (task 3.2): MVP's only implemented environment
//! provider. Activation runs once, host-side, before any sandbox
//! restriction (design.md decision 2) — this module never runs inside the
//! boundary and never activates per session.

use super::{Provider, ProviderError, Resolution};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Nix/flox's default store root. Overridden per-resolution when the
/// activated `PATH` names a different one (see [`store_grants`]).
const DEFAULT_STORE_ROOT: &str = "/nix/store";

/// What `flox activate` runs with — see [`capture_activated_env`]. A
/// conventional Linux default `PATH` (Debian's, notably): flox's own
/// hook scripts (`mkdir`, `cd`, etc.) need *some* POSIX baseline to
/// bootstrap from, same as any shell script assumes `/bin/sh` exists;
/// this does not depend on it being exactly this list, just *a* sane one,
/// fixed rather than inherited.
const CANONICAL_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

pub struct FloxProvider;

impl Provider for FloxProvider {
    /// Resolve a flox environment: verify `.flox/` exists, run `flox
    /// activate -- env -0` once to capture the post-activation environment,
    /// diff it against the same fixed base activation ran with, and derive
    /// the read-only store grants the compiled policy must carry.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_environment_present(project_root)?;

        let baseline = canonical_base_env()?;
        let activated = capture_activated_env(project_root, &baseline)?;

        Ok(Resolution {
            env: diff_env(&baseline, &activated),
            read_only_grants: store_grants(&activated),
        })
    }
}

/// The fixed environment `flox activate` runs with, and the baseline
/// [`diff_env`] compares against — the same map for both, so the diff
/// reflects exactly what activation changed and nothing else.
///
/// Found via review: this used to be `std::env::vars()` — literally
/// whatever shell happened to run `up` — for both roles. A `PATH` full of
/// a particular operator's own tools (nvm, rustup, cargo, ad hoc `~/bin`,
/// ...) leaking into either side means the exact same manifest can
/// resolve a different activation diff depending on who ran `up` and from
/// which shell, which is exactly the kind of non-reproducibility
/// CLAUDE.md's "no non-reproducible mode" framing rule rules out — the
/// manifest and lockfile are supposed to be the entire input.
///
/// `HOME` stays real: flox's own state/cache/credentials legitimately
/// live there and are user-specific by design, not something this
/// determinism guarantee is about. Everything else is fixed.
fn canonical_base_env() -> Result<BTreeMap<String, String>, ProviderError> {
    let home = std::env::var("HOME").map_err(|_| {
        ProviderError::ResolutionFailed("HOME is not set; flox needs it to activate".to_string())
    })?;
    Ok(BTreeMap::from([
        ("HOME".to_string(), home),
        ("PATH".to_string(), CANONICAL_PATH.to_string()),
    ]))
}

/// Resolves `name` to an absolute path by searching *this process's own*
/// `PATH` — i.e. wherever this host actually installed `flox`, which has
/// no reason to be under [`CANONICAL_PATH`] (a devcontainer feature, a
/// package manager, a user install — anywhere). Done before the
/// activation subprocess's own environment is fixed in
/// [`capture_activated_env`] and deliberately kept separate from it:
/// `std::process::Command`'s own bare-name resolution searches whatever
/// `PATH` is configured *on the command* at spawn time (confirmed: with
/// `.env_clear()` + a fixed `PATH` already applied, `Command::new("flox")`
/// fails to find a real `flox` binary living outside that fixed list) —
/// not the parent process's ambient one. Resolving here, once, up front,
/// keeps `flox` findable wherever this host put it while still letting
/// the child's own environment be fully fixed.
fn resolve_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        is_executable_file(&candidate).then_some(candidate)
    })
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
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

fn capture_activated_env(
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ProviderError> {
    let flox_bin = resolve_on_path("flox").ok_or(ProviderError::MissingBinary)?;

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

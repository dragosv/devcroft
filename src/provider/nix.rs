//! The `nix` provider (add-nix-provider): resolves a nix flake's default
//! dev shell, host-side, before any sandbox restriction (design.md
//! decision 2) — same contract as `flox.rs`, sharing its capture/diff
//! machinery via `provider::capture`. Preconditions run in a fixed order
//! (design.md decision 5) so a failure always names the most specific fix
//! available rather than surfacing nix's own, less specific error first.

use super::capture;
use super::{Provider, ProviderError, Resolution};
use crate::paths::resolve_on_path;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct NixProvider;

impl Provider for NixProvider {
    /// Resolve a nix flake's default dev shell: verify `flake.nix` exists,
    /// then `nix`, then `flake.lock`, then probe evaluability, then run
    /// `nix develop --command sh -c 'env -0 > <tmp>'` once to capture the
    /// activated environment, diff it against the same fixed baseline
    /// `flox.rs` uses, and derive the read-only store grants.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_flake_present(project_root)?;
        let nix_bin = resolve_on_path("nix").ok_or(ProviderError::MissingBinary {
            provider: "nix",
            hint: "devcroft doctor",
        })?;
        ensure_lock_present(project_root)?;
        probe_flake_evaluable(&nix_bin, project_root)?;

        let baseline = capture::canonical_base_env()?;
        let activated = capture_activated_env(&nix_bin, project_root, &baseline)?;

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: capture::store_grants(&activated),
            // Nix flakes have no service concept. Declared explicitly
            // rather than left as an empty list: `up` must be able to
            // tell "this provider cannot do services" from "this
            // provider can, and none are declared", so a project asking
            // for services here fails loudly instead of silently
            // starting nothing.
            services: super::ServiceSupport::Unsupported,
        })
    }
}

/// `up` fails at layer `provider` with the `nix flake init` hint (spec:
/// "Missing environment, not missing feature") rather than letting `nix
/// develop` produce its own, less specific error.
fn ensure_flake_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join("flake.nix").is_file() {
        Ok(())
    } else {
        Err(ProviderError::NoEnvironment {
            provider: "nix",
            hint: "nix flake init",
        })
    }
}

/// A `flake.nix` without `flake.lock` has nothing pinning its inputs —
/// resolving it would mean the same manifest can produce a different
/// closure depending on when `up` ran (design.md decision 3). Checked
/// before the evaluability probe so the hint is precise rather than
/// whatever generic message nix's own missing-lock handling would give
/// under `--no-update-lock-file`.
fn ensure_lock_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join("flake.lock").is_file() {
        Ok(())
    } else {
        Err(ProviderError::MissingLock {
            provider: "nix",
            hint: "nix flake lock",
        })
    }
}

/// One cheap `nix flake metadata` call that covers three distinct failure
/// modes at once (design.md decision 5): flakes disabled, daemon
/// unreachable, and a lockfile that doesn't cover the flake's current
/// inputs. Runs with `--no-update-lock-file` so an out-of-date lock fails
/// loudly here rather than nix silently repinning it.
fn probe_flake_evaluable(nix_bin: &Path, project_root: &Path) -> Result<(), ProviderError> {
    let output = Command::new(nix_bin)
        .arg("flake")
        .arg("metadata")
        .arg("--no-update-lock-file")
        .arg(project_root)
        .output()
        .map_err(|e| {
            ProviderError::ResolutionFailed(format!("running `nix flake metadata`: {e}"))
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(classify_metadata_failure(&String::from_utf8_lossy(
        &output.stderr,
    )))
}

/// nix's own error text is not a stable API — this is best-effort
/// substring matching to turn a generic evaluation failure into the more
/// specific hint the spec asks for, falling back to the raw message when
/// none of the known shapes match. Never claims more precision than it
/// has: an unrecognized failure stays a generic [`ProviderError::ResolutionFailed`]
/// rather than being forced into one of the three named categories.
fn classify_metadata_failure(stderr: &str) -> ProviderError {
    if stderr.contains("experimental Nix feature") || stderr.contains("is disabled") {
        return ProviderError::ResolutionFailed(format!(
            "nix flakes are not enabled; add `experimental-features = nix-command flakes` \
             to nix.conf (see `devcroft doctor`): {}",
            stderr.trim()
        ));
    }
    if stderr.contains("cannot connect to daemon") || stderr.contains("daemon-socket") {
        return ProviderError::ResolutionFailed(format!(
            "cannot reach the nix daemon: {}",
            stderr.trim()
        ));
    }
    if stderr.contains("lock file")
        && (stderr.contains("out of date")
            || stderr.contains("does not match")
            || stderr.contains("requires"))
    {
        return ProviderError::MissingLock {
            provider: "nix",
            hint: "nix flake lock",
        };
    }
    ProviderError::ResolutionFailed(format!("`nix flake metadata` failed: {}", stderr.trim()))
}

fn capture_activated_env(
    nix_bin: &Path,
    project_root: &Path,
    base: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ProviderError> {
    // `env -0`'s stdout is redirected to a file inside the `--command`
    // script rather than captured as the child process's own stdout, so
    // that a dev shell's `shellHook` printing to stdout (common — status
    // messages, banners) can never corrupt the capture: shellHook chatter
    // goes to `nix develop`'s real stdout, which this discards, while only
    // the redirected `env -0` dump is ever read back (design.md risk 2).
    let dump_path = scratch_dump_path();
    let script = format!("env -0 > '{}'", dump_path.display());

    let output = Command::new(nix_bin)
        .arg("develop")
        .arg(project_root)
        .arg("--no-update-lock-file")
        .arg("--no-write-lock-file")
        .arg("--command")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .current_dir(project_root)
        .env_clear()
        .envs(base)
        .output()
        .map_err(|e| ProviderError::ResolutionFailed(format!("running `nix develop`: {e}")))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&dump_path);
        return Err(ProviderError::ResolutionFailed(format!(
            "`nix develop` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let raw = std::fs::read(&dump_path).map_err(|e| {
        ProviderError::ResolutionFailed(format!(
            "reading captured environment dump {}: {e}",
            dump_path.display()
        ))
    })?;
    let _ = std::fs::remove_file(&dump_path);

    Ok(capture::parse_env_dump(&raw))
}

/// A scratch path for the `env -0` capture (see [`capture_activated_env`]),
/// unique enough to survive concurrent `up`s of different sandboxes on the
/// same host: pid plus a nanosecond timestamp, not a security boundary —
/// just collision avoidance for a file that lives for one `nix develop`
/// invocation.
fn scratch_dump_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "devcroft-nix-envdump-{}-{nanos}.tmp",
        std::process::id()
    ))
}

/// Content fingerprint of a nix flake's `flake.nix` + `flake.lock`, for
/// staleness detection (spec: "Stale environment after flake change").
/// `provider::is_stale` (dispatched by provider name) compares this
/// against the fingerprint recorded at the last `up`.
pub fn flake_fingerprint(project_root: &Path) -> Result<String, ProviderError> {
    ensure_flake_present(project_root)?;
    let flake_path = project_root.join("flake.nix");
    let flake = std::fs::read(&flake_path).map_err(|e| {
        ProviderError::ResolutionFailed(format!("reading {}: {e}", flake_path.display()))
    })?;
    // The lock does not exist until `nix flake lock` runs; its absence is
    // itself part of what makes a fingerprint change once one appears —
    // same reasoning as flox's manifest.lock handling.
    let lock = std::fs::read(project_root.join("flake.lock")).unwrap_or_default();

    Ok(capture::fingerprint(&[&flake, &lock]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("devcroft-nix-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A minimal, real flake: a `devShells.<system>.default` exporting one
    /// distinctive env var, built entirely from nixpkgs' own bootstrap
    /// derivations so it needs no network fetch beyond the pinned input
    /// itself once `flake.lock` exists. Enumerates the common systems
    /// statically rather than reading `builtins.currentSystem` — nix
    /// flakes evaluate pure by default (this provider deliberately never
    /// passes `--impure`, design.md decision 3) and `currentSystem` is not
    /// available under pure evaluation; the `nix develop` CLI itself picks
    /// the matching `devShells.<system>` entry for whichever host runs it.
    const FLAKE_NIX: &str = r#"
{
  description = "devcroft nix provider test fixture";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      shellFor = system: (import nixpkgs { inherit system; }).mkShell {
        DEVCROFT_NIX_TEST = "present";
      };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
"#;

    fn real_nix() -> Option<PathBuf> {
        resolve_on_path("nix").filter(|nix| {
            Command::new(nix)
                .arg("flake")
                .arg("--help")
                .output()
                .is_ok_and(|o| o.status.success())
        })
    }

    #[test]
    fn resolve_fails_with_no_environment_when_flake_missing() {
        let root = tempdir("no-flake");
        let err = NixProvider.resolve(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "nix",
                hint: "nix flake init"
            }
        );
    }

    #[test]
    fn resolve_fails_with_missing_lock_when_flake_lock_absent() {
        // Requires a real `nix` on PATH: the missing-binary check runs
        // before the lock check (design.md decision 5's ordering), so
        // without nix this would report MissingBinary instead — exactly
        // as intended, but not what this test is about.
        if real_nix().is_none() {
            return;
        }
        let root = tempdir("no-lock");
        fs::write(root.join("flake.nix"), FLAKE_NIX).unwrap();
        let err = NixProvider.resolve(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::MissingLock {
                provider: "nix",
                hint: "nix flake lock"
            }
        );
    }

    #[test]
    fn classify_metadata_failure_recognizes_disabled_flakes() {
        let err = classify_metadata_failure(
            "error: experimental Nix feature 'flakes' is disabled; use '--extra-experimental-features flakes' to override",
        );
        match err {
            ProviderError::ResolutionFailed(msg) => assert!(msg.contains("experimental-features")),
            other => panic!("expected ResolutionFailed naming the config fix, got {other:?}"),
        }
    }

    #[test]
    fn classify_metadata_failure_recognizes_unreachable_daemon() {
        let err = classify_metadata_failure(
            "error: cannot connect to daemon at '/nix/var/nix/daemon-socket/socket'",
        );
        match err {
            ProviderError::ResolutionFailed(msg) => assert!(msg.contains("daemon")),
            other => panic!("expected ResolutionFailed naming the daemon, got {other:?}"),
        }
    }

    #[test]
    fn classify_metadata_failure_recognizes_stale_lock() {
        let err = classify_metadata_failure(
            "error: 'flake.lock' file is out of date; lock file entry does not match",
        );
        assert_eq!(
            err,
            ProviderError::MissingLock {
                provider: "nix",
                hint: "nix flake lock"
            }
        );
    }

    #[test]
    fn classify_metadata_failure_falls_back_to_generic_message() {
        let err = classify_metadata_failure("error: something nix-specific and unrecognized");
        match err {
            ProviderError::ResolutionFailed(msg) => {
                assert!(msg.contains("nix flake metadata"));
                assert!(msg.contains("unrecognized"));
            }
            other => panic!("expected generic ResolutionFailed, got {other:?}"),
        }
    }

    #[test]
    fn flake_fingerprint_changes_when_flake_nix_changes() {
        let root = tempdir("fingerprint-flake");
        fs::write(root.join("flake.nix"), "{ description = \"a\"; }").unwrap();
        fs::write(root.join("flake.lock"), "locked-a").unwrap();
        let before = flake_fingerprint(&root).unwrap();

        fs::write(root.join("flake.nix"), "{ description = \"b\"; }").unwrap();
        let after = flake_fingerprint(&root).unwrap();

        assert_ne!(before, after);
    }

    #[test]
    fn flake_fingerprint_is_stable_for_unchanged_content() {
        let root = tempdir("fingerprint-stable");
        fs::write(root.join("flake.nix"), "{ description = \"a\"; }").unwrap();
        fs::write(root.join("flake.lock"), "locked-a").unwrap();

        let first = flake_fingerprint(&root).unwrap();
        let second = flake_fingerprint(&root).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn flake_fingerprint_fails_without_flake() {
        let root = tempdir("fingerprint-missing");
        let err = flake_fingerprint(&root).unwrap_err();
        assert_eq!(
            err,
            ProviderError::NoEnvironment {
                provider: "nix",
                hint: "nix flake init"
            }
        );
    }

    #[test]
    fn resolve_against_a_real_flake_if_nix_is_available() {
        // Best-effort integration check, same pattern as flox.rs's own:
        // only runs meaningfully where `nix` is installed with flakes
        // enabled. Skips quietly otherwise so the suite stays portable.
        let Some(nix) = real_nix() else { return };
        let root = tempdir("real-resolve");
        fs::write(root.join("flake.nix"), FLAKE_NIX).unwrap();

        let lock = Command::new(&nix)
            .arg("flake")
            .arg("lock")
            .arg(&root)
            .output()
            .unwrap();
        if !lock.status.success() {
            // No network reachable to resolve nixpkgs, or similar
            // environment limitation unrelated to what this test checks —
            // skip rather than fail the whole suite over connectivity.
            return;
        }

        let resolution = NixProvider.resolve(&root).unwrap();
        assert_eq!(
            resolution.env.get("DEVCROFT_NIX_TEST").map(String::as_str),
            Some("present")
        );
        assert!(!resolution.read_only_grants.is_empty());
    }
}

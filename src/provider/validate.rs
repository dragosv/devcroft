//! Rejects every `env.provider` value that isn't a supported declarative
//! environment provider, with a message that distinguishes *why*: "out of
//! scope by design" (no reproducibility at all), "version manager" (fails
//! the completeness/lockfile/precondition tests in docs/decisions.md §1),
//! or "not yet supported" (on the roadmap, not built). See the
//! `env-provider` spec's "Only declarative providers" requirement.

use super::ProviderError;

const OUT_OF_SCOPE_REASON: &str = "devcroft has no non-reproducible mode; every sandbox is backed \
    by a declarative environment. Run `flox init`, not a degraded mode.";
const OUT_OF_SCOPE: &[&str] = &["host", "none"];

const VERSION_MANAGER_REASON: &str = "version managers install imperatively and cannot guarantee \
    a restorable lockfile, a complete build environment, and preconditions verifiable at `up`; \
    see docs/decisions.md §1";
const VERSION_MANAGERS: &[&str] = &[
    "rustup", "nvm", "pyenv", "rbenv", "sdkman", "ghcup", "asdf", "proto",
];

const NOT_YET_SUPPORTED: &[(&str, &str)] = &[
    (
        "nix",
        "nix flakes support is planned (closure tier) but not yet implemented",
    ),
    (
        "flake",
        "nix flakes support is planned (closure tier) but not yet implemented",
    ),
    (
        "flakes",
        "nix flakes support is planned (closure tier) but not yet implemented",
    ),
    (
        "devbox",
        "devbox support is planned (closure tier) but not yet implemented",
    ),
    (
        "mise",
        "mise support is planned (artifact tier) but not yet implemented",
    ),
    (
        "devenv",
        "devenv is a qualified provider (closure tier) but not yet scheduled",
    ),
    (
        "pixi",
        "pixi is a qualified provider (artifact tier) but not yet scheduled",
    ),
    (
        "hermit",
        "hermit is a qualified provider (artifact tier) but not yet scheduled",
    ),
];

/// Validate an `env.provider` value. `Ok(())` only for `"flox"`, MVP's
/// sole supported provider.
pub fn validate_provider(name: &str) -> Result<(), ProviderError> {
    if name == "flox" {
        return Ok(());
    }
    if OUT_OF_SCOPE.contains(&name) {
        return Err(ProviderError::OutOfScope {
            name: name.to_string(),
            reason: OUT_OF_SCOPE_REASON,
        });
    }
    if VERSION_MANAGERS.contains(&name) {
        return Err(ProviderError::VersionManager {
            name: name.to_string(),
            reason: VERSION_MANAGER_REASON,
        });
    }
    if let Some((_, reason)) = NOT_YET_SUPPORTED.iter().find(|(n, _)| *n == name) {
        return Err(ProviderError::NotYetSupported {
            name: name.to_string(),
            reason,
        });
    }
    Err(ProviderError::Unknown {
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flox_is_accepted() {
        assert!(validate_provider("flox").is_ok());
    }

    #[test]
    fn host_and_none_are_out_of_scope() {
        for name in ["host", "none"] {
            match validate_provider(name) {
                Err(ProviderError::OutOfScope { name: got, .. }) => assert_eq!(got, name),
                other => panic!("expected OutOfScope for {name}, got {other:?}"),
            }
        }
    }

    #[test]
    fn version_managers_are_rejected() {
        for name in VERSION_MANAGERS {
            match validate_provider(name) {
                Err(ProviderError::VersionManager { name: got, .. }) => assert_eq!(&got, name),
                other => panic!("expected VersionManager for {name}, got {other:?}"),
            }
        }
    }

    #[test]
    fn mise_reports_planned_artifact_tier() {
        match validate_provider("mise") {
            Err(ProviderError::NotYetSupported { reason, .. }) => {
                assert!(reason.contains("planned"));
                assert!(reason.contains("artifact tier"));
                assert!(reason.contains("not yet implemented"));
            }
            other => panic!("expected NotYetSupported, got {other:?}"),
        }
    }

    #[test]
    fn nix_flakes_reports_planned_closure_tier() {
        for name in ["nix", "flake", "flakes"] {
            match validate_provider(name) {
                Err(ProviderError::NotYetSupported { reason, .. }) => {
                    assert!(reason.contains("closure tier"));
                }
                other => panic!("expected NotYetSupported for {name}, got {other:?}"),
            }
        }
    }

    #[test]
    fn unrecognized_name_is_unknown() {
        match validate_provider("totally-made-up") {
            Err(ProviderError::Unknown { name }) => assert_eq!(name, "totally-made-up"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}

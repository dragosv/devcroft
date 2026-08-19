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

/// Provider names devcroft actually resolves. `flake`/`flakes` are
/// accepted aliases for `nix` — normalized by [`normalize_provider_name`]
/// so exactly one canonical name ever reaches provider dispatch, `status`,
/// and policy rule origins.
const SUPPORTED: &[&str] = &["flox", "nix", "flake", "flakes"];

const NOT_YET_SUPPORTED: &[(&str, &str)] = &[
    (
        "devbox",
        "devbox support is planned (closure tier) but not yet implemented",
    ),
    (
        "mise",
        "mise is a qualified provider (artifact tier) but not yet scheduled",
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

/// Validate an `env.provider` value. `Ok(())` for `"flox"`, `"nix"`, and
/// nix's `"flake"`/`"flakes"` aliases.
pub fn validate_provider(name: &str) -> Result<(), ProviderError> {
    if SUPPORTED.contains(&name) {
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

/// Normalize an already-`validate_provider`-accepted name to the single
/// canonical form provider dispatch, `status`, and policy rule origins
/// use. Only `nix`'s aliases fold today; every other accepted name is
/// already canonical.
pub fn normalize_provider_name(name: &str) -> String {
    match name {
        "flake" | "flakes" => "nix".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flox_is_accepted() {
        assert!(validate_provider("flox").is_ok());
    }

    #[test]
    fn nix_and_its_aliases_are_accepted() {
        for name in ["nix", "flake", "flakes"] {
            assert!(validate_provider(name).is_ok(), "{name} should validate");
        }
    }

    #[test]
    fn nix_aliases_normalize_to_nix() {
        assert_eq!(normalize_provider_name("nix"), "nix");
        assert_eq!(normalize_provider_name("flake"), "nix");
        assert_eq!(normalize_provider_name("flakes"), "nix");
    }

    #[test]
    fn flox_name_is_unaffected_by_normalization() {
        assert_eq!(normalize_provider_name("flox"), "flox");
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

    /// mise is qualified but unscheduled, not planned: `add-mise-provider`
    /// was removed when `own-policy-baseline` established that the
    /// baseline grants no host library paths, so an artifact-tier provider
    /// has to declare those grants itself. The six criteria it meets are
    /// unchanged — meeting them simply stopped implying host access. The
    /// message must not promise a schedule devcroft no longer has.
    #[test]
    fn mise_reports_qualified_but_unscheduled_artifact_tier() {
        match validate_provider("mise") {
            Err(ProviderError::NotYetSupported { reason, .. }) => {
                assert!(reason.contains("qualified"));
                assert!(reason.contains("artifact tier"));
                assert!(reason.contains("not yet scheduled"));
                assert!(
                    !reason.contains("planned"),
                    "no change is scheduled for mise; the message must not imply one"
                );
            }
            other => panic!("expected NotYetSupported, got {other:?}"),
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

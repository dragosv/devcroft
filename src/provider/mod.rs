//! The `env-provider` capability: resolves the declarative environment
//! (toolchain, PATH, env vars) a manifest names, host-side, before any
//! sandbox restriction applies (design.md decision 2). Implemented
//! providers: flox (task 3.2) and nix flakes (add-nix-provider). This
//! module also validates `env.provider` against every other name devcroft
//! is ever going to support, rejecting the rest up front with a message
//! naming why, and dispatches the validated name to the right
//! implementation — the one place both `resolve` and staleness
//! fingerprinting are keyed off the provider name, so adding a third
//! provider touches this file once rather than every call site.

mod capture;
mod flox;
mod nix;
mod validate;

pub use flox::FloxProvider;
pub use nix::NixProvider;
pub use validate::{normalize_provider_name, validate_provider};

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Resolves a declarative environment host-side, before any sandbox
/// restriction is applied. Implementations run once at `up`; sessions
/// inherit the captured result for free (design.md decision 2).
pub trait Provider {
    /// Capture the activation env diff and the read-only store paths that
    /// must be granted for the resolved toolchain to work.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError>;
}

/// What a provider's activation produced: the environment diff to inject
/// into the keeper, the read-only paths the compiled policy must grant for
/// the resolved toolchain to run, and any baseline variable activation
/// explicitly removed rather than changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Resolution {
    pub env: BTreeMap<String, String>,
    /// Baseline keys activation's output no longer has at all — distinct
    /// from `env`, whose map type can only represent "set to this value",
    /// never "unset". The keeper's process otherwise inherits `up`'s own
    /// ambient environment (see `lifecycle::up::spawn_keeper`), so without
    /// this, a variable activation removes would still leak through from
    /// whoever's shell happened to run `up` — the exact non-reproducibility
    /// `canonical_base_env` was already introduced to close for changed
    /// keys (found during review, previously undetected for removed ones).
    pub unset: Vec<String>,
    pub read_only_grants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// `env.provider` names something devcroft will never support —
    /// there is no non-reproducible mode.
    OutOfScope { name: String, reason: &'static str },
    /// `env.provider` names a single-ecosystem toolchain manager, which
    /// cannot deliver devcroft's reproducibility guarantees.
    VersionManager { name: String, reason: &'static str },
    /// `env.provider` names a provider on the roadmap but not yet built.
    NotYetSupported { name: String, reason: &'static str },
    /// `env.provider` names something devcroft has no record of at all.
    Unknown { name: String },
    /// The provider binary (e.g. `flox`, `nix`) is not on `PATH`.
    MissingBinary {
        provider: &'static str,
        hint: &'static str,
    },
    /// The project has no provider environment to activate (e.g. no
    /// `.flox/`, no `flake.nix`) — a missing environment, not a missing
    /// feature.
    NoEnvironment {
        provider: &'static str,
        hint: &'static str,
    },
    /// The provider has an environment definition but no lockfile pinning
    /// it (e.g. `flake.nix` without `flake.lock`) — distinct from
    /// [`ProviderError::NoEnvironment`] because the fix is "lock it", not
    /// "create it".
    MissingLock {
        provider: &'static str,
        hint: &'static str,
    },
    /// The provider's own resolution failed (activation error, unreadable
    /// manifest, etc).
    ResolutionFailed(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::OutOfScope { name, reason } => {
                write!(f, "provider `{name}` is out of scope: {reason}")
            }
            ProviderError::VersionManager { name, reason } => {
                write!(f, "provider `{name}` is not supported: {reason}")
            }
            ProviderError::NotYetSupported { name, reason } => {
                write!(f, "provider `{name}` is not yet supported: {reason}")
            }
            ProviderError::Unknown { name } => {
                write!(
                    f,
                    "unknown provider `{name}`; devcroft supports `flox` and `nix` in this release"
                )
            }
            ProviderError::MissingBinary { provider, hint } => write!(
                f,
                "`{provider}` is not installed or not on PATH; run `{hint}`"
            ),
            ProviderError::NoEnvironment { provider, hint } => write!(
                f,
                "no {provider} environment found in this project; run `{hint}`"
            ),
            ProviderError::MissingLock { provider, hint } => write!(
                f,
                "no lockfile found for the {provider} environment; run `{hint}`"
            ),
            ProviderError::ResolutionFailed(msg) => write!(f, "provider resolution failed: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// The validated, canonical provider names devcroft can actually resolve.
/// Everything downstream that needs to run *a* provider (as opposed to
/// merely validating the manifest's `env.provider` string) goes through
/// this — `up`'s activation capture and `status`'s staleness check both
/// dispatch off the same enum, so a third provider is added here once
/// rather than at every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Flox,
    Nix,
}

impl ProviderKind {
    /// `name` must already be validated and normalized (see
    /// [`validate_provider`], [`normalize_provider_name`]) — this only
    /// ever sees `"flox"` or `"nix"` in practice, since `config::parse` is
    /// the sole place a `Manifest` is constructed. Returns
    /// [`ProviderError::Unknown`] rather than panicking so a manifest
    /// built by a test or another caller that skipped validation still
    /// fails through the normal error contract instead of crashing.
    pub fn from_name(name: &str) -> Result<Self, ProviderError> {
        match name {
            "flox" => Ok(ProviderKind::Flox),
            "nix" => Ok(ProviderKind::Nix),
            other => Err(ProviderError::Unknown {
                name: other.to_string(),
            }),
        }
    }
}

impl Provider for ProviderKind {
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        match self {
            ProviderKind::Flox => FloxProvider.resolve(project_root),
            ProviderKind::Nix => NixProvider.resolve(project_root),
        }
    }
}

/// Content fingerprint of the environment definition `provider` names, for
/// staleness detection — dispatches to the matching provider's own
/// fingerprint (flox: `manifest.toml` + lockfile; nix: `flake.nix` +
/// `flake.lock`).
pub fn manifest_fingerprint(provider: &str, project_root: &Path) -> Result<String, ProviderError> {
    match ProviderKind::from_name(provider)? {
        ProviderKind::Flox => flox::manifest_fingerprint(project_root),
        ProviderKind::Nix => nix::flake_fingerprint(project_root),
    }
}

/// Whether `provider`'s environment has changed since `recorded` was
/// captured (spec: "Stale environment after manifest/flake change").
pub fn is_stale(
    provider: &str,
    project_root: &Path,
    recorded: &str,
) -> Result<bool, ProviderError> {
    Ok(manifest_fingerprint(provider, project_root)? != recorded)
}

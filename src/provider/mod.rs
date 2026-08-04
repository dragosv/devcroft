//! The `env-provider` capability: resolves the declarative environment
//! (toolchain, PATH, env vars) a manifest names, host-side, before any
//! sandbox restriction applies (design.md decision 2). MVP's only
//! implemented provider is flox (task 3.2); this module also validates
//! `env.provider` against every other name devcroft is ever going to
//! support, rejecting the rest up front with a message naming why.

mod flox;
mod validate;

pub use flox::{FloxProvider, is_stale, manifest_fingerprint};
pub use validate::validate_provider;

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
/// into the keeper, and the read-only paths the compiled policy must
/// grant for the resolved toolchain to run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Resolution {
    pub env: BTreeMap<String, String>,
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
    /// The provider binary (e.g. `flox`) is not on `PATH`.
    MissingBinary,
    /// The project has no provider environment to activate (e.g. no
    /// `.flox/`) — a missing environment, not a missing feature.
    NoEnvironment,
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
                    "unknown provider `{name}`; devcroft supports `flox` in this release"
                )
            }
            ProviderError::MissingBinary => write!(
                f,
                "`flox` is not installed or not on PATH; run `devcroft doctor`"
            ),
            ProviderError::NoEnvironment => write!(
                f,
                "no flox environment found in this project; run `flox init`"
            ),
            ProviderError::ResolutionFailed(msg) => write!(f, "provider resolution failed: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

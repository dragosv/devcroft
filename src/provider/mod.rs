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
    /// Store paths this provider's resolved toolchain needs read access
    /// to, compiled into the policy with a `provider:<name>` origin
    /// (`CompiledPolicy::with_provider_grants`) and shown by
    /// `policy --render` under that name.
    ///
    /// **The baseline grants no host library paths** (own-policy-baseline:
    /// `KEEPER_SYSTEM_READ` covers only what the keeper itself needs,
    /// never anything for project code). A closure-tier provider (flox,
    /// nix, devbox) needs nothing here beyond its own store root — the
    /// closure supplies its own linker and C library. A provider whose
    /// runtime links against *host* libraries (mise, pixi, hermit —
    /// `docs/decisions.md` §1's artifact tier) must declare those host
    /// paths here explicitly, or its resolved environment will fail to
    /// execute rather than silently working by inheriting access the
    /// baseline used to grant implicitly. This is the artifact tier's
    /// defining trade-off made visible in the compiled policy rather than
    /// resting on a tier name in documentation.
    pub read_only_grants: Vec<String>,
    /// What this provider has to say about long-lived services. Captured
    /// host-side with the rest of resolution — the declarations are read
    /// from the provider's own manifest, never by running project code.
    pub services: ServiceSupport,
}

/// A provider's service story. Deliberately three-valued rather than a
/// bare `Vec`: "this provider has no service concept" and "this provider
/// supports services and none are declared" are different facts, and
/// collapsing them into an empty list is what would let a manifest
/// asking for services under `nix` silently start nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ServiceSupport {
    /// The provider has no service mechanism at all (nix flakes today).
    #[default]
    Unsupported,
    /// The provider supports services; zero or more are declared.
    Declared(Vec<ServiceDecl>),
}

impl ServiceSupport {
    /// The declared services, or an empty slice when unsupported —
    /// for callers that only need to iterate. Callers that must
    /// *distinguish* unsupported from empty (the `up` precondition
    /// check) match on the enum instead.
    pub fn declared(&self) -> &[ServiceDecl] {
        match self {
            ServiceSupport::Unsupported => &[],
            ServiceSupport::Declared(v) => v,
        }
    }
}

/// One service the provider's manifest declares. `command` is project
/// code and is only ever executed inside the sandbox, after restriction
/// — never during resolution.
///
/// The fields mirror flox's documented `[services]` schema, which is the
/// contract this depends on (deliberately, over flox's *undocumented*
/// generated `service-config.yaml` — see design.md decision 1). Dropping
/// any of them is a correctness bug, not a simplification: a service
/// whose port comes from `vars` starts on the wrong port if `vars` is
/// ignored, and a daemon reaped without its `shutdown` command is killed
/// rather than stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDecl {
    pub name: String,
    pub command: String,
    /// Environment variables scoped to this service alone, layered over
    /// the sandbox's captured environment.
    pub vars: BTreeMap<String, String>,
    /// flox's `is-daemon`: the command backgrounds itself instead of
    /// staying in the foreground. Such a service cannot be supervised by
    /// watching the spawned process — it exits immediately by design —
    /// and must be stopped via [`Self::shutdown_command`].
    pub is_daemon: bool,
    /// flox's `shutdown.command`: how to stop a daemon service. Required
    /// in practice whenever `is_daemon` is set, since killing the
    /// (already-exited) launcher stops nothing.
    pub shutdown_command: Option<String>,
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

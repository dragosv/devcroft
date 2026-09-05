mod discovery;
mod slug;
mod validate;

pub use discovery::{MANIFEST_FILE_NAME, discover};
pub use slug::{is_valid_name, slugify};

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// The declarative sandbox manifest (`devcroft.toml`), fully resolved:
/// every optional field carries its documented default and
/// `sandbox.name` is guaranteed present and valid.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub sandbox: Sandbox,
    pub env: Env,
    pub filesystem: Filesystem,
    pub network: Network,
    pub ssh: Ssh,
    pub hooks: Hooks,
    /// `[[broker]]` entries, in declaration order.
    pub brokers: Vec<Broker>,
}

/// One brokered upstream: the sandbox reaches it through a local route the
/// proxy owns, and the credential stays on the host (`adopt-nono-proxy`,
/// `brokered-credentials`).
///
/// Declared by **provider**, not by agent (design D5): `provider` becomes the
/// route prefix, from which the client-facing variables are derived
/// (`anthropic` → `ANTHROPIC_BASE_URL`), so any client following that
/// provider's SDK convention is brokered without devcroft naming it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Broker {
    /// Route prefix, naming the upstream API rather than the client.
    pub provider: String,
    /// Where the proxy forwards to, e.g. `https://api.anthropic.com`.
    pub upstream: String,
    /// An **indirection** to the secret, never the secret itself — a manifest
    /// is committed. Resolved host-side at `up`, in the trusted phase (D4).
    pub secret: String,
    /// Overrides the variable name derived from `provider`, for an SDK that
    /// does not follow the `{PREFIX}_API_KEY` convention (D5).
    #[serde(default)]
    pub env_var: Option<String>,
    /// The header the credential is injected into upstream.
    ///
    /// Defaults per provider — see [`Broker::inject_header`] — because the two
    /// large providers disagree: Anthropic reads `x-api-key`, OpenAI reads
    /// `Authorization`. Overridable because devcroft cannot know every API's
    /// convention and guessing wrong would be unfixable from a manifest.
    #[serde(default)]
    pub header: Option<String>,
}

impl Broker {
    /// The header this route injects into.
    ///
    /// The table is deliberately tiny and covers only what devcroft can be
    /// sure of. Everything else falls through to `Authorization`, for which
    /// `nono-proxy` builds `Bearer {}` — any other header name gets the bare
    /// secret, which is what `x-api-key` wants. So the header name is the only
    /// per-provider fact devcroft has to carry.
    ///
    /// **Revisit if** this table grows past a handful of entries: at that
    /// point it is provider data, not a default, and belongs somewhere a user
    /// can extend without a devcroft release.
    pub fn inject_header(&self) -> String {
        if let Some(h) = &self.header {
            return h.clone();
        }
        match self.provider.as_str() {
            "anthropic" => "x-api-key".to_string(),
            _ => "Authorization".to_string(),
        }
    }

    /// The variable an SDK reads its key from — the phantom token goes here.
    pub fn key_var(&self) -> String {
        self.env_var
            .clone()
            .unwrap_or_else(|| format!("{}_API_KEY", self.provider.to_uppercase()))
    }

    /// The variable an SDK reads its base URL from.
    pub fn base_url_var(&self) -> String {
        format!("{}_BASE_URL", self.provider.to_uppercase())
    }

    /// The host variable this route's secret is read *from*, when `secret`
    /// uses the `env:` scheme.
    ///
    /// Needed for one reason, found by `tests/broker_credential_injection.rs`
    /// rather than by review: the user must have the credential exported for
    /// `env:NAME` to resolve at all, and a provider's activated environment
    /// carries devcroft's own ambient variables through to the sandbox. So the
    /// secret arrived inside by plain inheritance, on a path that has nothing
    /// to do with the route — defeating brokering with its own precondition.
    /// `up` scrubs this name before handing the environment to the keeper.
    pub fn source_var(&self) -> Option<&str> {
        self.secret.strip_prefix("env:")
    }

    /// The env var the proxy process reads this route's secret from. Namespaced
    /// so devcroft's own resolution (and its empty-value rule) stays the single
    /// path, rather than the manifest's `env:` name becoming an implicit
    /// contract with the child's inherited environment.
    pub fn secret_var(&self) -> String {
        format!("DEVCROFT_BROKER_SECRET_{}", self.provider.to_uppercase())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sandbox {
    pub name: String,
}

/// The one isolation tier devcroft provides.
///
/// `remove-gvisor-backend` collapsed the axis: there was a `hardened`
/// tier backed by gVisor, and it was removed because Landlock cannot
/// mediate `mount()` — which `runsc` requires — so the two could not be
/// stacked at all, and because the tier sat in a squeezed middle between
/// a cheaper process tier and a stronger VM. `[sandbox].isolation` still
/// parses, so a manifest naming the removed tier gets a message that says
/// so rather than a generic unknown-value error (see
/// [`ConfigError::RemovedIsolationTier`]); it just has one valid value.
pub const ISOLATION_TIER: &str = "process";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Env {
    pub provider: String,
    pub vars: BTreeMap<String, String>,
}

impl Default for Env {
    fn default() -> Self {
        Env {
            provider: "flox".to_string(),
            vars: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Filesystem {
    pub allow: Vec<String>,
    pub read: Vec<String>,
    pub deny: Vec<String>,
}

impl Default for Filesystem {
    fn default() -> Self {
        Filesystem {
            allow: vec![".".to_string()],
            read: Vec::new(),
            deny: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Network {
    pub default: NetworkDefault,
    pub allow: Vec<String>,
    /// Loopback TCP ports the sandbox may bind *and* connect on, granted
    /// independently of `default`/`allow` — those two govern **outbound**
    /// egress, this governs local listeners.
    ///
    /// Without this, `default = "deny"` denies `bind`/`listen` outright,
    /// including on loopback, so a dev server or a database inside the
    /// sandbox cannot come up at all; the only workaround was
    /// `default = "allow"`, which restores binding by dropping egress
    /// filtering entirely. That was documented as a gap in the policy
    /// model itself — it is not: nono's profile schema has always had the
    /// field, devcroft simply never emitted it (see
    /// `policy::NonoNetwork::open_port`).
    ///
    /// Loopback only, and explicit ports only. Confirmed live against
    /// nono 0.71.0 on Linux: a profile with `block: true` plus
    /// `open_port` binds `127.0.0.1:<port>` successfully, while the
    /// neighboring `listen_port` field grants neither a loopback nor a
    /// `0.0.0.0` bind on this platform — so this maps to `open_port`, and
    /// binding a non-loopback address stays denied.
    pub ports: Vec<u16>,
}

impl Default for Network {
    fn default() -> Self {
        Network {
            default: NetworkDefault::Deny,
            allow: Vec::new(),
            ports: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkDefault {
    Deny,
    Allow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct Ssh {
    pub forward_agent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct Hooks {
    pub post_create: Option<String>,
    pub post_start: Option<String>,
}

/// Deserialization target used before mandatory-field checks run;
/// `sandbox.name` is optional here so a missing name produces
/// [`ConfigError::MissingName`] instead of a generic serde error.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawManifest {
    sandbox: RawSandbox,
    env: Env,
    filesystem: Filesystem,
    network: Network,
    ssh: Ssh,
    hooks: Hooks,
    #[serde(default, rename = "broker")]
    brokers: Vec<Broker>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawSandbox {
    name: Option<String>,
    /// Raw, so a removed or unknown tier produces devcroft's own message
    /// rather than serde's "unknown variant".
    isolation: Option<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    NotFound,
    Io(std::io::Error),
    Parse(String),
    UnknownKey {
        path: String,
        suggestion: Option<String>,
    },
    MissingName,
    /// A `[[broker]]` entry names an upstream the manifest has not allowed.
    /// Refused rather than implicitly granted: a route that widened
    /// `network.allow` on its own would break "nothing goes to the backend
    /// that cannot be shown via `policy --render`".
    BrokerUpstreamNotAllowed {
        provider: String,
        host: String,
    },
    /// A `[[broker]]` field is empty or malformed.
    InvalidBroker {
        provider: String,
        field: &'static str,
        detail: String,
    },
    InvalidName {
        name: String,
        suggestion: String,
    },
    InvalidPath {
        field: &'static str,
        value: String,
    },
    UselessDeny {
        path: String,
    },
    InvalidProvider(crate::provider::ProviderError),
    /// `[sandbox].isolation` names the tier `remove-gvisor-backend`
    /// removed. Its own variant rather than a generic invalid-value
    /// error, because the spec requires the message to name the removed
    /// tier, the supported one, and the path to a stronger boundary.
    RemovedIsolationTier {
        name: String,
    },
    /// `[sandbox].isolation` names something that never existed.
    InvalidIsolationTier {
        name: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NotFound => write!(
                f,
                "no {MANIFEST_FILE_NAME} found in this directory or its ancestors; run `devcroft init`"
            ),
            ConfigError::Io(e) => write!(f, "reading manifest: {e}"),
            ConfigError::BrokerUpstreamNotAllowed { provider, host } => write!(
                f,
                "broker `{provider}` forwards to `{host}`, which `network.allow` does not permit\n\
                 \x20 add it: network.allow = [\"{host}\"]\n\
                 \x20 the proxy dials the upstream for the sandbox, so a route cannot reach further \
                 than the manifest already allows"
            ),
            ConfigError::InvalidBroker {
                provider,
                field,
                detail,
            } => write!(f, "broker `{provider}`: `{field}` {detail}"),
            ConfigError::Parse(e) => write!(f, "invalid TOML: {e}"),
            ConfigError::UnknownKey { path, suggestion } => {
                write!(f, "unknown key `{path}`")?;
                if let Some(s) = suggestion {
                    write!(f, "; did you mean `{s}`?")?;
                }
                Ok(())
            }
            ConfigError::MissingName => write!(f, "`[sandbox].name` is required"),
            ConfigError::InvalidName { name, suggestion } => write!(
                f,
                "invalid sandbox name `{name}`: must match [a-z0-9][a-z0-9-]{{0,31}}; try `{suggestion}`"
            ),
            ConfigError::InvalidPath { field, value } => {
                write!(f, "invalid path in `filesystem.{field}`: `{value}`")
            }
            ConfigError::UselessDeny { path } => write!(
                f,
                "`filesystem.deny` entry `{path}` is never granted by `allow` or `read`"
            ),
            ConfigError::InvalidProvider(e) => write!(f, "{e}"),
            ConfigError::RemovedIsolationTier { name } => write!(
                f,
                "`[sandbox].isolation = \"{name}\"` names a tier devcroft no longer \
                 provides; the supported tier is `{ISOLATION_TIER}`. For a boundary \
                 stronger than the process tier, run devcroft inside a VM — that is \
                 the supported path, and is already how the macOS path works"
            ),
            ConfigError::InvalidIsolationTier { name } => write!(
                f,
                "`[sandbox].isolation = \"{name}\"` is not a known isolation tier; \
                 the supported tier is `{ISOLATION_TIER}`"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// `filesystem.allow`/`read` grants a known credential directory.
    SensitivePath { field: &'static str, path: String },
    /// An `[env] vars` value looks like it expects host interpolation,
    /// which devcroft never performs.
    NoInterpolation,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Warning::SensitivePath { field, path } => write!(
                f,
                "`filesystem.{field}` grants `{path}`, a known credential directory"
            ),
            Warning::NoInterpolation => write!(
                f,
                "`[env] vars` values are literal strings; host environment variables are not interpolated"
            ),
        }
    }
}

/// Discover and load the manifest starting from `start` and its ancestors.
pub fn load(start: &Path) -> Result<(Manifest, Vec<Warning>), ConfigError> {
    let manifest_path = discover(start).map_err(|_| ConfigError::NotFound)?;
    let text = std::fs::read_to_string(&manifest_path).map_err(ConfigError::Io)?;
    parse(&text)
}

/// Parse and validate manifest text directly (used by tests and `init`'s
/// preview/dry-run path).
pub fn parse(text: &str) -> Result<(Manifest, Vec<Warning>), ConfigError> {
    let table = text
        .parse::<toml::Table>()
        .map_err(|e| ConfigError::Parse(e.to_string()))?;
    validate::check_unknown_keys(&table)?;

    let raw: RawManifest = toml::Value::Table(table)
        .try_into()
        .map_err(|e| ConfigError::Parse(e.to_string()))?;

    let name = raw.sandbox.name.ok_or(ConfigError::MissingName)?;
    if !is_valid_name(&name) {
        return Err(ConfigError::InvalidName {
            suggestion: slugify(&name),
            name,
        });
    }

    crate::provider::validate_provider(&raw.env.provider).map_err(ConfigError::InvalidProvider)?;
    // A manifest that omits the key is fine and gets no output at all —
    // the spec's "Manifest omits the isolation level" scenario rules out
    // a deprecation notice for the common case.
    match raw.sandbox.isolation.as_deref() {
        None => {}
        Some(t) if t == ISOLATION_TIER => {}
        Some("hardened") => {
            return Err(ConfigError::RemovedIsolationTier {
                name: "hardened".to_string(),
            });
        }
        Some(other) => {
            return Err(ConfigError::InvalidIsolationTier {
                name: other.to_string(),
            });
        }
    }

    validate::check_filesystem(&raw.filesystem)?;
    validate::check_brokers(&raw.brokers, &raw.network)?;

    let mut warnings = Vec::new();
    validate::collect_warnings(&raw.env, &raw.filesystem, &mut warnings);

    // `flake`/`flakes` are accepted aliases for `nix` (validated above);
    // normalized here so exactly one canonical name ever reaches provider
    // dispatch, `status` output, and policy rule origins.
    let mut env = raw.env;
    env.provider = crate::provider::normalize_provider_name(&env.provider);

    Ok((
        Manifest {
            sandbox: Sandbox { name },
            env,
            filesystem: raw.filesystem,
            network: raw.network,
            ssh: raw.ssh,
            hooks: raw.hooks,
            brokers: raw.brokers,
        },
        warnings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A brokered route parses, and is declared by *provider* rather than by
    /// agent — `adopt-nono-proxy` D5. `anthropic` here is the upstream API's
    /// name, not Claude Code's.
    #[test]
    fn a_broker_route_parses_and_is_keyed_by_provider() {
        let (m, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["api.anthropic.com"]
            [[broker]]
            provider = "anthropic"
            upstream = "https://api.anthropic.com"
            secret = "env:ANTHROPIC_API_KEY"
            "#,
        )
        .unwrap();
        assert_eq!(m.brokers.len(), 1);
        assert_eq!(m.brokers[0].provider, "anthropic");
        assert_eq!(m.brokers[0].env_var, None);
    }

    /// The rule that keeps a route from widening the policy behind the
    /// reader's back: the proxy dials the upstream on the sandbox's behalf, so
    /// an unallowed host would be egress `policy --render` never shows.
    #[test]
    fn a_broker_upstream_outside_network_allow_is_refused() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["crates.io"]
            [[broker]]
            provider = "anthropic"
            upstream = "https://api.anthropic.com"
            secret = "env:ANTHROPIC_API_KEY"
            "#,
        )
        .unwrap_err();
        match err {
            ConfigError::BrokerUpstreamNotAllowed { provider, host } => {
                assert_eq!(provider, "anthropic");
                assert_eq!(host, "api.anthropic.com");
            }
            other => panic!("expected BrokerUpstreamNotAllowed, got {other:?}"),
        }
    }

    /// The control for the test above — without it, a check that refused
    /// *everything* would pass it.
    #[test]
    fn a_wildcard_allow_covers_its_broker_upstream() {
        let (m, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["*.anthropic.com"]
            [[broker]]
            provider = "anthropic"
            upstream = "https://api.anthropic.com/v1"
            secret = "env:ANTHROPIC_API_KEY"
            "#,
        )
        .unwrap();
        assert_eq!(m.brokers.len(), 1);
    }

    /// `[[broker]]` is the schema's only array of tables, so it is the one
    /// shape where an unchecked field would be silently accepted.
    #[test]
    fn an_unknown_broker_field_is_reported_with_its_index() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["api.anthropic.com"]
            [[broker]]
            provider = "anthropic"
            upstream = "https://api.anthropic.com"
            secret = "env:K"
            upstrem = "typo"
            "#,
        )
        .unwrap_err();
        match err {
            ConfigError::UnknownKey { path, suggestion } => {
                assert_eq!(path, "broker[0].upstrem");
                assert_eq!(suggestion.as_deref(), Some("upstream"));
            }
            other => panic!("expected UnknownKey, got {other:?}"),
        }
    }

    /// The prefix becomes both a URL path segment and an environment-variable
    /// stem, so it cannot be arbitrary text.
    #[test]
    fn a_broker_provider_name_is_constrained() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["api.anthropic.com"]
            [[broker]]
            provider = "anthropic/v1"
            upstream = "https://api.anthropic.com"
            secret = "env:K"
            "#,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidBroker {
                field: "provider",
                ..
            }
        ));
    }

    #[test]
    fn a_broker_upstream_must_be_an_absolute_http_url() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            allow = ["api.anthropic.com"]
            [[broker]]
            provider = "anthropic"
            upstream = "api.anthropic.com"
            secret = "env:K"
            "#,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidBroker {
                field: "upstream",
                ..
            }
        ));
    }

    #[test]
    fn minimal_manifest_gets_defaults() {
        let (m, warnings) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        assert_eq!(m.sandbox.name, "myproj");
        assert_eq!(m.env.provider, "flox");
        assert_eq!(m.filesystem.allow, vec!["."]);
        assert_eq!(m.network.default, NetworkDefault::Deny);
        assert!(m.hooks.post_create.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn missing_name_is_an_error() {
        let err = parse("").unwrap_err();
        assert!(matches!(err, ConfigError::MissingName));
    }

    #[test]
    fn unknown_top_level_key_suggests_section() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [netwrk]
            default = "deny"
            "#,
        )
        .unwrap_err();
        match err {
            ConfigError::UnknownKey { path, suggestion } => {
                assert_eq!(path, "netwrk");
                assert_eq!(suggestion.as_deref(), Some("network"));
            }
            other => panic!("expected UnknownKey, got {other:?}"),
        }
    }

    #[test]
    fn unknown_nested_key_reports_full_path() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            nmae = "typo"
            "#,
        )
        .unwrap_err();
        match err {
            ConfigError::UnknownKey { path, suggestion } => {
                assert_eq!(path, "sandbox.nmae");
                assert_eq!(suggestion.as_deref(), Some("name"));
            }
            other => panic!("expected UnknownKey, got {other:?}"),
        }
    }

    #[test]
    fn env_vars_keys_are_not_schema_checked() {
        let (m, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [env.vars]
            ANYTHING_GOES = "1"
            "#,
        )
        .unwrap();
        assert_eq!(
            m.env.vars.get("ANYTHING_GOES").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn invalid_name_reports_slug_suggestion() {
        let err = parse("[sandbox]\nname = \"My Project\"\n").unwrap_err();
        match err {
            ConfigError::InvalidName { name, suggestion } => {
                assert_eq!(name, "My Project");
                assert_eq!(suggestion, "my-project");
            }
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn deny_within_allow_succeeds() {
        let (m, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["~"]
            deny = ["~/.ssh"]
            "#,
        )
        .unwrap();
        assert_eq!(m.filesystem.deny, vec!["~/.ssh"]);
    }

    #[test]
    fn deny_outside_any_grant_is_useless() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["src"]
            deny = ["docs"]
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::UselessDeny { path } if path == "docs"));
    }

    #[test]
    fn traversal_in_allow_is_rejected() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["../../etc"]
            "#,
        )
        .unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidPath { field, value } if field == "allow" && value == "../../etc")
        );
    }

    #[test]
    fn traversal_in_read_is_rejected() {
        let err = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            read = ["src/../../secrets"]
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::InvalidPath { field, .. } if field == "read"));
    }

    #[test]
    fn traversal_under_home_or_absolute_is_also_rejected() {
        // `..` breaks is_within's containment model (deny-wins-over-allow,
        // sensitive-path warnings, baseline-deny-unless-granted) regardless
        // of which root it's rooted under, not just the project root.
        for value in ["~/../../etc", "/etc/../root"] {
            let err = parse(&format!(
                "[sandbox]\nname = \"myproj\"\n[filesystem]\nallow = [{value:?}]\n"
            ))
            .unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidPath { .. }),
                "expected {value:?} to be rejected, got {err:?}"
            );
        }
    }

    #[test]
    fn a_literal_dotdot_looking_prefix_is_not_a_traversal() {
        // Component-based, not substring-based: a directory that merely
        // starts with ".." (e.g. "..bak") is a real, unambiguous name and
        // must not be rejected.
        let (manifest, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["..bak"]
            "#,
        )
        .unwrap();
        assert_eq!(manifest.filesystem.allow, vec!["..bak".to_string()]);
    }

    #[test]
    fn sensitive_allow_warns_but_succeeds() {
        let (_, warnings) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [filesystem]
            allow = ["~/.ssh"]
            "#,
        )
        .unwrap();
        assert!(warnings.contains(&Warning::SensitivePath {
            field: "allow",
            path: "~/.ssh".to_string(),
        }));
    }

    #[test]
    fn interpolation_looking_value_warns() {
        let (m, warnings) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [env.vars]
            TOKEN = "$HOST_TOKEN"
            "#,
        )
        .unwrap();
        assert_eq!(
            m.env.vars.get("TOKEN").map(String::as_str),
            Some("$HOST_TOKEN")
        );
        assert!(warnings.contains(&Warning::NoInterpolation));
    }

    #[test]
    fn provider_override_leaves_default() {
        let (m, _) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            default = "allow"
            allow = ["github.com"]
            "#,
        )
        .unwrap();
        assert_eq!(m.network.default, NetworkDefault::Allow);
        assert_eq!(m.network.allow, vec!["github.com"]);
    }

    #[test]
    fn network_ports_parse_and_default_to_empty() {
        let (bare, _) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        assert!(
            bare.network.ports.is_empty(),
            "omitting the key must grant no ports"
        );

        let (m, warnings) = parse(
            r#"
            [sandbox]
            name = "myproj"
            [network]
            default = "deny"
            ports = [5432, 6379]
            "#,
        )
        .unwrap();
        // The combination that matters: egress denied *and* local ports
        // granted. These are independent axes, not a contradiction — the
        // whole point of the key.
        assert_eq!(m.network.default, NetworkDefault::Deny);
        assert_eq!(m.network.ports, vec![5432, 6379]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn out_of_range_port_is_a_config_error() {
        // 65536 does not fit u16; this must fail as a config error rather
        // than wrapping to 0 (which nono would read as "any port").
        assert!(parse("[sandbox]\nname = \"m\"\n[network]\nports = [65536]\n").is_err());
    }

    #[test]
    fn host_provider_is_rejected_as_config_error() {
        let err = parse("[sandbox]\nname = \"myproj\"\n[env]\nprovider = \"host\"\n").unwrap_err();
        match err {
            ConfigError::InvalidProvider(crate::provider::ProviderError::OutOfScope {
                name,
                ..
            }) => assert_eq!(name, "host"),
            other => panic!("expected InvalidProvider(OutOfScope), got {other:?}"),
        }
    }

    #[test]
    fn mise_provider_is_rejected_as_not_yet_supported() {
        let err = parse("[sandbox]\nname = \"myproj\"\n[env]\nprovider = \"mise\"\n").unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidProvider(crate::provider::ProviderError::NotYetSupported { .. })
        ));
    }

    /// `add-test-runtime-fixture` introduces an internal seam that lets
    /// tests drive `up` with a provider of their own. This asserts the half
    /// of that which must stay *impossible*: the seam is an internal API,
    /// not a schema extension, so no fixture name may become selectable
    /// from a manifest (`provider-injection-seam`: "not reachable from a
    /// manifest or the published binary").
    ///
    /// Worth a test rather than a comment because the pressure to relax it
    /// is real and arrives later — once a fixture exists, making it
    /// nameable is a one-line change that would silently reintroduce the
    /// passthrough provider devcroft does not have.
    #[test]
    fn a_fixture_name_is_not_selectable_from_a_manifest() {
        for name in ["test", "fixture", "testprov"] {
            let err = parse(&format!(
                "[sandbox]\nname = \"myproj\"\n[env]\nprovider = {name:?}\n"
            ))
            .unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidProvider(_)),
                "provider {name:?} must be rejected by the parser, got {err:?}"
            );
        }
    }

    #[test]
    fn nix_provider_is_accepted() {
        let (m, _) = parse("[sandbox]\nname = \"myproj\"\n[env]\nprovider = \"nix\"\n").unwrap();
        assert_eq!(m.env.provider, "nix");
    }

    /// Spec: "Manifest omits the isolation level" — the supported tier is
    /// used, and **no deprecation output is produced**. A manifest that
    /// never mentioned the removed tier should not learn it existed.
    #[test]
    fn omitting_isolation_parses_with_no_warning() {
        let (_, warnings) = parse("[sandbox]\nname = \"myproj\"\n").unwrap();
        assert!(
            warnings.is_empty(),
            "omitting the key must produce no output at all, got {warnings:?}"
        );
    }

    #[test]
    fn the_supported_tier_parses() {
        assert!(
            parse(&format!(
                "[sandbox]\nname = \"myproj\"\nisolation = \"{ISOLATION_TIER}\"\n"
            ))
            .is_ok()
        );
    }

    /// Spec: "Manifest selects a removed tier" — the message names the
    /// removed tier, the supported tier, and the VM path, and there is no
    /// silent fallback. Asserted on the rendered text because the whole
    /// point of the dedicated variant is what the user reads.
    #[test]
    fn the_removed_hardened_tier_is_named_not_merely_rejected() {
        let err = parse("[sandbox]\nname = \"myproj\"\nisolation = \"hardened\"\n").unwrap_err();
        assert!(matches!(err, ConfigError::RemovedIsolationTier { .. }));
        let msg = err.to_string();
        assert!(
            msg.contains("hardened"),
            "must name the removed tier: {msg}"
        );
        assert!(
            msg.contains(ISOLATION_TIER),
            "must name the supported tier: {msg}"
        );
        assert!(
            msg.contains("VM"),
            "must name the stronger-boundary path: {msg}"
        );
    }

    /// A tier that never existed is distinguishable from the one that was
    /// removed — telling a typo it was "removed" would be a small lie.
    #[test]
    fn an_unknown_tier_is_not_reported_as_removed() {
        let err = parse("[sandbox]\nname = \"myproj\"\nisolation = \"vm\"\n").unwrap_err();
        assert!(matches!(err, ConfigError::InvalidIsolationTier { .. }));
    }

    #[test]
    fn nix_aliases_normalize_to_canonical_name() {
        for alias in ["flake", "flakes"] {
            let (m, _) = parse(&format!(
                "[sandbox]\nname = \"myproj\"\n[env]\nprovider = \"{alias}\"\n"
            ))
            .unwrap();
            assert_eq!(m.env.provider, "nix", "alias `{alias}` should normalize");
        }
    }
}

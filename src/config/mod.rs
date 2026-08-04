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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sandbox {
    pub name: String,
}

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
}

impl Default for Network {
    fn default() -> Self {
        Network {
            default: NetworkDefault::Deny,
            allow: Vec::new(),
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
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawSandbox {
    name: Option<String>,
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
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NotFound => write!(
                f,
                "no {MANIFEST_FILE_NAME} found in this directory or its ancestors; run `devcroft init`"
            ),
            ConfigError::Io(e) => write!(f, "reading manifest: {e}"),
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

    validate::check_filesystem(&raw.filesystem)?;

    let mut warnings = Vec::new();
    validate::collect_warnings(&raw.env, &raw.filesystem, &mut warnings);

    Ok((
        Manifest {
            sandbox: Sandbox { name },
            env: raw.env,
            filesystem: raw.filesystem,
            network: raw.network,
            ssh: raw.ssh,
            hooks: raw.hooks,
        },
        warnings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(m.env.vars.get("ANYTHING_GOES").map(String::as_str), Some("1"));
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
        assert_eq!(m.env.vars.get("TOKEN").map(String::as_str), Some("$HOST_TOKEN"));
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
}

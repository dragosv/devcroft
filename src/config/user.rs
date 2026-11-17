//! The user's own configuration (`add-agent-workload` design.md Decision 1):
//! `$XDG_CONFIG_HOME/devcroft/config.toml`, or `~/.config/devcroft/config.toml`
//! when that is unset, on every platform.
//!
//! **devcroft's first user configuration, and deliberately a small one.** It
//! holds what belongs to the operator rather than to the project — the
//! tooling layer and the credentials it needs — and nothing that changes how a
//! project's own manifest is read. Everything else stays in `devcroft.toml`, so "what does this project
//! do" is still answered by the repository.
//!
//! Validated as strictly as `devcroft.toml`, and for the same reason: a key
//! that parses and does nothing is the failure this project refuses to ship.
//! That is also why a credential has exactly two accepted shapes: delivered as
//! an environment variable (`credentials.env`), or brokered through the egress
//! proxy (`[[credentials.broker]]`), where the sandbox holds a placeholder. The
//! single-file shape waits on a measurement (`add-agent-workload` task 4.0),
//! and until something delivers it, `credentials.file` is an unknown key, not
//! a silent one.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::ConfigError;
use super::validate::closest;

/// The file's path under the configuration directory.
pub const USER_CONFIG_FILE: &str = "devcroft/config.toml";

/// The parsed user configuration. Absent file and empty file are the same:
/// no layer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserConfig {
    pub tools: Option<ToolsDecl>,
    /// `[credentials] env`: variables whose values come from the invoking
    /// shell into every sandbox this user brings up, disclosed at each `up`
    /// (`credentials` spec). The user's counterpart to a manifest's
    /// `[env] forward`: the key an agent needs is the operator's, so the
    /// project's committed file need not name it.
    pub credentials: Vec<String>,
    /// `[[credentials.broker]]`: credentials the egress proxy injects, so
    /// the sandbox holds a placeholder and never the value
    /// (`add-agent-workload` design.md Decision 6).
    pub brokers: Vec<BrokerDecl>,
}

/// One brokered credential. The real value is read from the user's shell
/// at `up`, under `env`, and handed to the proxy alone; the sandbox gets a
/// placeholder under the same name, and the proxy's endpoint for `host`
/// under `endpoint_env`. Holds no value, so `Meta` records it as it is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrokerDecl {
    /// The upstream, reached as `https://<host>`. A bare name: no scheme,
    /// port, path or wildcard.
    pub host: String,
    /// The request header that carries the key, e.g. `x-api-key`.
    pub header: String,
    /// The variable the placeholder is delivered in, and the one the real
    /// value is read from in the invoking shell.
    pub env: String,
    /// The variable the sandbox gets `http://127.0.0.1:<port>/<prefix>` in.
    pub endpoint_env: String,
    /// A `nono-proxy` `phantom_format` (`ghp_{}`), for a client that checks
    /// a key's shape before sending it. Absent, the placeholder is the bare
    /// session token.
    pub placeholder: Option<String>,
}

/// `[tools]`: the directory holding the user's tooling environment, and,
/// optionally, which provider resolves it. Absent, the provider is detected
/// from the directory, the way `up --tools` always does
/// (`crate::tooling::detect_provider`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsDecl {
    pub provider: Option<String>,
    /// Absolute, `~` already expanded.
    pub path: PathBuf,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawUserConfig {
    tools: Option<RawTools>,
    credentials: RawCredentials,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawCredentials {
    env: Vec<String>,
    broker: Vec<RawBroker>,
}

#[derive(Deserialize)]
struct RawBroker {
    host: String,
    header: String,
    env: String,
    endpoint_env: String,
    placeholder: Option<String>,
}

#[derive(Deserialize)]
struct RawTools {
    provider: Option<String>,
    path: String,
}

const SECTIONS: &[(&str, &[&str])] = &[
    ("tools", &["provider", "path"]),
    ("credentials", &["env", "broker"]),
];

/// The keys of one `[[credentials.broker]]` entry.
const BROKER_FIELDS: &[&str] = &["host", "header", "env", "endpoint_env", "placeholder"];

/// Variables devcroft or the proxy sets itself in every sandbox with egress,
/// which a broker would otherwise silently overwrite or be overwritten by.
const RESERVED_VARS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "HOME",
    "PATH",
];

/// Where the user configuration lives on this host, or `None` when neither
/// `XDG_CONFIG_HOME` nor `HOME` says. A relative `XDG_CONFIG_HOME` is ignored,
/// as the XDG specification requires.
pub fn user_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(USER_CONFIG_FILE))
}

/// Read and validate the user configuration. A missing file is not an
/// error; anything wrong with one that exists is, naming the file.
pub fn load_user_config() -> Result<UserConfig, ConfigError> {
    let Some(file) = user_config_path() else {
        return Ok(UserConfig::default());
    };
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(UserConfig::default()),
        Err(e) => return Err(in_file(&file, ConfigError::Io(e))),
    };
    parse_user_config(&text, &home()).map_err(|e| in_file(&file, e))
}

/// Parse user configuration text, expanding a leading `~` against `home`.
pub fn parse_user_config(text: &str, home: &Path) -> Result<UserConfig, ConfigError> {
    let table = text
        .parse::<toml::Table>()
        .map_err(|e| ConfigError::Parse(e.to_string()))?;
    check_unknown_keys(&table)?;
    let raw: RawUserConfig = toml::Value::Table(table)
        .try_into()
        .map_err(|e| ConfigError::Parse(e.to_string()))?;
    let tools = match raw.tools {
        None => None,
        Some(t) => Some(ToolsDecl {
            // Validated here, at layer `config`, as `devcroft.toml`'s
            // `env.provider` is; whether it is closure-tier is the layer's
            // own question, asked at `up` (`crate::tooling::select`).
            provider: match t.provider {
                Some(p) => {
                    crate::provider::validate_provider(&p).map_err(ConfigError::InvalidProvider)?;
                    Some(crate::provider::normalize_provider_name(&p))
                }
                None => None,
            },
            path: expand_path(&t.path, home).ok_or_else(|| {
                ConfigError::Parse(format!(
                    "tools.path: {:?} must be absolute or start with `~/`; a relative path \
                     has nothing to be relative to",
                    t.path
                ))
            })?,
        }),
    };
    let mut credentials = Vec::new();
    for name in raw.credentials.env {
        if !super::is_env_var_name(&name) {
            return Err(ConfigError::Parse(format!(
                "credentials.env: {name:?} is not an environment variable name (letters, \
                 digits and `_`, not starting with a digit)"
            )));
        }
        if !credentials.contains(&name) {
            credentials.push(name);
        }
    }
    let brokers = parse_brokers(raw.credentials.broker, &credentials)?;
    Ok(UserConfig {
        tools,
        credentials,
        brokers,
    })
}

/// Shape checks for `[[credentials.broker]]`. What needs the project (the
/// allowlist) or the shell (the value) is checked at `up`.
fn parse_brokers(
    raw: Vec<RawBroker>,
    env_credentials: &[String],
) -> Result<Vec<BrokerDecl>, ConfigError> {
    let err = |i: usize, key: &str, msg: String| {
        ConfigError::Parse(format!("credentials.broker[{i}].{key}: {msg}"))
    };
    let mut out: Vec<BrokerDecl> = Vec::new();
    for (i, b) in raw.into_iter().enumerate() {
        if !is_bare_host(&b.host) {
            return Err(err(
                i,
                "host",
                format!(
                    "{:?} must be a bare host name such as `api.anthropic.com`: no scheme, \
                     port, path or wildcard, since the proxy reaches it as `https://<host>`",
                    b.host
                ),
            ));
        }
        if b.header.is_empty() || !b.header.bytes().all(is_header_name_byte) {
            return Err(err(
                i,
                "header",
                format!("{:?} is not an HTTP header name", b.header),
            ));
        }
        for (key, name) in [("env", &b.env), ("endpoint_env", &b.endpoint_env)] {
            if !super::is_env_var_name(name) {
                return Err(err(
                    i,
                    key,
                    format!(
                        "{name:?} is not an environment variable name (letters, digits and \
                         `_`, not starting with a digit)"
                    ),
                ));
            }
            let upper = name.to_ascii_uppercase();
            if RESERVED_VARS.contains(&upper.as_str()) || upper.starts_with("DEVCROFT_") {
                return Err(err(
                    i,
                    key,
                    format!("`{name}` is set by devcroft itself in every sandbox"),
                ));
            }
        }
        if b.env == b.endpoint_env {
            return Err(err(
                i,
                "endpoint_env",
                format!(
                    "`{}` is also this entry's `env`; the placeholder and the endpoint need a variable each",
                    b.env
                ),
            ));
        }
        // Brokered and delivered at once would put the value in the sandbox
        // after all, which is the one thing brokering exists to prevent.
        if env_credentials.contains(&b.env) {
            return Err(err(
                i,
                "env",
                format!(
                    "`{}` is also in `credentials.env`, which delivers the real value into the \
                     sandbox; a credential is brokered or delivered, not both",
                    b.env
                ),
            ));
        }
        for prev in &out {
            for name in [&b.env, &b.endpoint_env] {
                if *name == prev.env || *name == prev.endpoint_env {
                    return Err(err(
                        i,
                        if *name == b.env {
                            "env"
                        } else {
                            "endpoint_env"
                        },
                        format!("`{name}` is already used by an earlier broker entry"),
                    ));
                }
            }
        }
        if let Some(fmt) = &b.placeholder {
            nono_proxy::token::PhantomTemplate::parse_route(fmt)
                .map_err(|e| err(i, "placeholder", e))?;
        }
        out.push(BrokerDecl {
            host: b.host.to_ascii_lowercase(),
            header: b.header,
            env: b.env,
            endpoint_env: b.endpoint_env,
            placeholder: b.placeholder,
        });
    }
    Ok(out)
}

/// A DNS name, lowercase or not: labels of letters, digits and `-`, at
/// least two of them. Rejects schemes, ports, paths and `*`.
fn is_bare_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// RFC 9110 `token` bytes.
fn is_header_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn check_unknown_keys(table: &toml::Table) -> Result<(), ConfigError> {
    let names: Vec<&str> = SECTIONS.iter().map(|(n, _)| *n).collect();
    for (key, value) in table {
        let Some((_, fields)) = SECTIONS.iter().find(|(n, _)| *n == key.as_str()) else {
            return Err(ConfigError::UnknownKey {
                path: key.clone(),
                suggestion: closest(key, &names),
            });
        };
        if let Some(sub) = value.as_table() {
            for sub_key in sub.keys() {
                if !fields.contains(&sub_key.as_str()) {
                    return Err(ConfigError::UnknownKey {
                        path: format!("{key}.{sub_key}"),
                        suggestion: closest(sub_key, fields),
                    });
                }
            }
            if let Some(entries) = sub.get("broker").and_then(|v| v.as_array()) {
                for (i, entry) in entries.iter().enumerate() {
                    for field in entry.as_table().into_iter().flat_map(|t| t.keys()) {
                        if !BROKER_FIELDS.contains(&field.as_str()) {
                            return Err(ConfigError::UnknownKey {
                                path: format!("{key}.broker[{i}].{field}"),
                                suggestion: closest(field, BROKER_FIELDS),
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn expand_path(raw: &str, home: &Path) -> Option<PathBuf> {
    if raw == "~" {
        return Some(home.to_path_buf());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    let path = PathBuf::from(raw);
    path.is_absolute().then_some(path)
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn in_file(file: &Path, error: ConfigError) -> ConfigError {
    ConfigError::InUserConfig {
        file: file.display().to_string(),
        error: Box::new(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/someone";

    fn parse(text: &str) -> Result<UserConfig, ConfigError> {
        parse_user_config(text, Path::new(HOME))
    }

    #[test]
    fn an_empty_file_declares_no_layer() {
        assert_eq!(parse("").unwrap(), UserConfig::default());
    }

    #[test]
    fn tools_parses_and_expands_home() {
        let c = parse("[tools]\nprovider = \"devenv\"\npath = \"~/agents\"\n").unwrap();
        assert_eq!(
            c.tools,
            Some(ToolsDecl {
                provider: Some("devenv".into()),
                path: PathBuf::from("/home/someone/agents"),
            })
        );
        let c = parse("[tools]\npath = \"/opt/agents\"\n").unwrap();
        assert_eq!(c.tools.unwrap().provider, None);
    }

    #[test]
    fn the_provider_is_validated_and_normalized() {
        let c = parse("[tools]\nprovider = \"flake\"\npath = \"/a\"\n").unwrap();
        assert_eq!(c.tools.unwrap().provider.as_deref(), Some("nix"));
        let err = parse("[tools]\nprovider = \"host\"\npath = \"/a\"\n").unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProvider(_)), "{err:?}");
    }

    #[test]
    fn a_relative_path_is_refused() {
        let err = parse("[tools]\npath = \"agents\"\n").unwrap_err();
        assert!(err.to_string().contains("tools.path"), "{err}");
    }

    /// Unknown keys are named by their full path, as in `devcroft.toml`.
    #[test]
    fn unknown_keys_are_named_with_their_path() {
        let err = parse("[tools]\npath = \"/a\"\nprovder = \"nix\"\n").unwrap_err();
        assert!(
            matches!(&err, ConfigError::UnknownKey { path, suggestion }
                if path == "tools.provder" && suggestion.as_deref() == Some("provider")),
            "{err:?}"
        );
        let err = parse("[tool]\npath = \"/a\"\n").unwrap_err();
        assert!(
            matches!(&err, ConfigError::UnknownKey { path, .. } if path == "tool"),
            "{err:?}"
        );
    }

    #[test]
    fn env_credentials_parse_and_are_checked() {
        let c =
            parse("[credentials]\nenv = [\"ANTHROPIC_API_KEY\", \"ANTHROPIC_API_KEY\"]\n").unwrap();
        assert_eq!(c.credentials, vec!["ANTHROPIC_API_KEY".to_string()]);
        let err = parse("[credentials]\nenv = [\"1BAD\"]\n").unwrap_err();
        assert!(err.to_string().contains("credentials.env"), "{err}");
    }

    /// The file shape is not accepted until something delivers it (task 4.0
    /// measures what it needs first): a request that parsed and did nothing
    /// would be read as working.
    #[test]
    fn a_file_credential_is_not_yet_a_key() {
        let err = parse("[credentials]\nfile = \"~/.claude/.credentials.json\"\n").unwrap_err();
        assert!(
            matches!(&err, ConfigError::UnknownKey { path, .. } if path == "credentials.file"),
            "{err:?}"
        );
    }

    const BROKER: &str = "[[credentials.broker]]\nhost = \"api.anthropic.com\"\n\
        header = \"x-api-key\"\nenv = \"ANTHROPIC_API_KEY\"\n\
        endpoint_env = \"ANTHROPIC_BASE_URL\"\n";

    #[test]
    fn a_broker_parses() {
        let c = parse(&format!("{BROKER}placeholder = \"sk-ant-{{}}\"\n")).unwrap();
        assert_eq!(
            c.brokers,
            vec![BrokerDecl {
                host: "api.anthropic.com".into(),
                header: "x-api-key".into(),
                env: "ANTHROPIC_API_KEY".into(),
                endpoint_env: "ANTHROPIC_BASE_URL".into(),
                placeholder: Some("sk-ant-{}".into()),
            }]
        );
        assert_eq!(parse(BROKER).unwrap().brokers[0].placeholder, None);
    }

    /// Each refusal names the entry and the key, as `devcroft.toml`'s do.
    #[test]
    fn a_broker_is_checked_key_by_key() {
        let bad = |from: &str, to: &str, key: &str| {
            let text = BROKER.replace(from, to);
            let err = parse(&text).unwrap_err().to_string();
            assert!(
                err.contains(&format!("credentials.broker[0].{key}")),
                "{from} -> {to}: {err}"
            );
        };
        bad(
            "\"api.anthropic.com\"",
            "\"https://api.anthropic.com\"",
            "host",
        );
        bad("\"api.anthropic.com\"", "\"api.anthropic.com:443\"", "host");
        bad("\"api.anthropic.com\"", "\"*.anthropic.com\"", "host");
        bad("\"api.anthropic.com\"", "\"localhost\"", "host");
        bad("\"x-api-key\"", "\"x api key\"", "header");
        bad("\"ANTHROPIC_API_KEY\"", "\"1KEY\"", "env");
        bad("\"ANTHROPIC_BASE_URL\"", "\"HTTPS_PROXY\"", "endpoint_env");
        bad(
            "\"ANTHROPIC_BASE_URL\"",
            "\"ANTHROPIC_API_KEY\"",
            "endpoint_env",
        );
        let err = parse(&format!("{BROKER}placeholder = \"Bearer {{}}\"\n"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("credentials.broker[0].placeholder"), "{err}");
    }

    /// Brokered and delivered at once would put the value in the sandbox.
    #[test]
    fn a_broker_cannot_share_a_variable_with_env_delivery_or_another_broker() {
        let err = parse(&format!(
            "[credentials]\nenv = [\"ANTHROPIC_API_KEY\"]\n{BROKER}"
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("credentials.broker[0].env"), "{err}");
        let err = parse(&format!("{BROKER}{BROKER}")).unwrap_err().to_string();
        assert!(err.contains("credentials.broker[1].env"), "{err}");
    }

    #[test]
    fn an_unknown_broker_key_is_named_with_its_entry() {
        let err = parse(&format!("{BROKER}hedaer = \"x\"\n")).unwrap_err();
        assert!(
            matches!(&err, ConfigError::UnknownKey { path, suggestion }
                if path == "credentials.broker[0].hedaer" && suggestion.as_deref() == Some("header")),
            "{err:?}"
        );
    }

    #[test]
    fn the_file_is_named_in_an_error() {
        let err = in_file(Path::new("/x/config.toml"), parse("[tool]\n").unwrap_err());
        let text = err.to_string();
        assert!(
            text.contains("/x/config.toml") && text.contains("`tool`"),
            "{text}"
        );
    }
}

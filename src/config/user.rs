//! The user's own configuration (`add-agent-workload` design.md Decision 1):
//! `$XDG_CONFIG_HOME/devcroft/config.toml`, or `~/.config/devcroft/config.toml`
//! when that is unset, on every platform.
//!
//! **devcroft's first user configuration, and deliberately a small one.** It
//! holds what belongs to the operator rather than to the project — today, the
//! tooling layer — and nothing that changes how a project's own manifest is
//! read. Everything else stays in `devcroft.toml`, so "what does this project
//! do" is still answered by the repository.
//!
//! Validated as strictly as `devcroft.toml`, and for the same reason: a key
//! that parses and does nothing is the failure this project refuses to ship.
//! That is also why the credential request this change specifies is not
//! accepted yet (task 2.4 waits for group 4): until something delivers it, a
//! `[credentials]` table is an unknown key, not a silent one.

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
}

#[derive(Deserialize)]
struct RawTools {
    provider: Option<String>,
    path: String,
}

const SECTIONS: &[(&str, &[&str])] = &[("tools", &["provider", "path"])];

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
    Ok(UserConfig { tools })
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

    /// Not accepted until something delivers it (task 2.4 waits for group
    /// 4): a credential request that parsed and did nothing would be read as
    /// working.
    #[test]
    fn a_credential_request_is_not_yet_a_key() {
        let err = parse("[credentials]\nenv = [\"ANTHROPIC_API_KEY\"]\n").unwrap_err();
        assert!(matches!(err, ConfigError::UnknownKey { .. }), "{err:?}");
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

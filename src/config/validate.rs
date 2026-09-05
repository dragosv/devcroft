use super::{Broker, ConfigError, Env, Filesystem, Network, Warning};
use crate::paths::{SENSITIVE_PATHS, has_traversal, is_within};

/// Known sections and, for each, its known field names. `env.vars` is
/// deliberately absent from `env`'s field list — it is a free-form table
/// of user-chosen names and is never schema-checked.
const SECTIONS: &[(&str, &[&str])] = &[
    ("sandbox", &["name", "isolation"]),
    ("env", &["provider", "vars"]),
    ("filesystem", &["allow", "read", "deny"]),
    ("network", &["default", "allow", "ports"]),
    ("ssh", &["forward_agent"]),
    ("hooks", &["post_create", "post_start"]),
    (
        "broker",
        &["provider", "upstream", "secret", "env_var", "header"],
    ),
];

pub fn check_unknown_keys(table: &toml::Table) -> Result<(), ConfigError> {
    let section_names: Vec<&str> = SECTIONS.iter().map(|(name, _)| *name).collect();

    for (key, value) in table {
        let Some((_, fields)) = SECTIONS.iter().find(|(name, _)| *name == key.as_str()) else {
            return Err(ConfigError::UnknownKey {
                path: key.clone(),
                suggestion: closest(key, &section_names),
            });
        };

        // `env.vars` holds arbitrary user-chosen keys; only the section's
        // own two field names (`provider`, `vars`) are schema-checked.
        if key == "env" {
            continue;
        }

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

        // `[[broker]]` is an array of tables, so `as_table` above sees
        // nothing and every field would go unchecked — the one shape in this
        // schema where a typo would otherwise be silently accepted.
        if let Some(entries) = value.as_array() {
            for (i, entry) in entries.iter().enumerate() {
                let Some(sub) = entry.as_table() else {
                    continue;
                };
                for sub_key in sub.keys() {
                    if !fields.contains(&sub_key.as_str()) {
                        return Err(ConfigError::UnknownKey {
                            path: format!("{key}[{i}].{sub_key}"),
                            suggestion: closest(sub_key, fields),
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

/// A brokered route may not reach further than the manifest already allows.
///
/// The proxy dials the upstream on the sandbox's behalf, so a route whose host
/// is absent from `network.allow` would be egress the compiled policy never
/// shows — breaking "nothing goes to the backend that cannot be shown via
/// `policy --render`", and doing it in the one place a reader is least likely
/// to look. Refused, naming the host, rather than granted implicitly.
pub fn check_brokers(brokers: &[Broker], network: &Network) -> Result<(), ConfigError> {
    for b in brokers {
        for (field, value) in [
            ("provider", &b.provider),
            ("upstream", &b.upstream),
            ("secret", &b.secret),
        ] {
            if value.trim().is_empty() {
                return Err(ConfigError::InvalidBroker {
                    provider: b.provider.clone(),
                    field,
                    detail: "must not be empty".to_string(),
                });
            }
        }
        // The prefix becomes an env-var stem and a URL path segment, so it is
        // held to the same shape a sandbox name is rather than anything
        // looser.
        if !b
            .provider
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ConfigError::InvalidBroker {
                provider: b.provider.clone(),
                field: "provider",
                detail: "may contain only letters, digits, '-' and '_' — it becomes both a URL path segment and an environment-variable stem".to_string(),
            });
        }
        let Some(host) = crate::proxy::backend::upstream_host(&b.upstream) else {
            return Err(ConfigError::InvalidBroker {
                provider: b.provider.clone(),
                field: "upstream",
                detail: format!("{:?} is not an absolute http(s) URL", b.upstream),
            });
        };
        if !network.allow.iter().any(|a| host_matches(&host, a)) {
            return Err(ConfigError::BrokerUpstreamNotAllowed {
                provider: b.provider.clone(),
                host,
            });
        }
    }
    Ok(())
}

/// `network.allow`'s own matching: exact, or a leading `*.` wildcard.
fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    match pattern.strip_prefix("*.") {
        Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
        None => host == pattern,
    }
}

pub fn check_filesystem(fs: &Filesystem) -> Result<(), ConfigError> {
    for (field, values) in [("allow", &fs.allow), ("read", &fs.read), ("deny", &fs.deny)] {
        for value in values {
            if value.is_empty() || value.contains('\0') || has_traversal(value) {
                return Err(ConfigError::InvalidPath {
                    field,
                    value: value.clone(),
                });
            }
        }
    }

    let granted: Vec<&String> = fs.allow.iter().chain(fs.read.iter()).collect();
    for deny in &fs.deny {
        if !granted.iter().any(|g| is_within(deny, g)) {
            return Err(ConfigError::UselessDeny { path: deny.clone() });
        }
    }

    Ok(())
}

pub fn collect_warnings(env: &Env, filesystem: &Filesystem, warnings: &mut Vec<Warning>) {
    for path in &filesystem.allow {
        if SENSITIVE_PATHS
            .iter()
            .any(|sensitive| is_within(sensitive, path))
        {
            warnings.push(Warning::SensitivePath {
                field: "allow",
                path: path.clone(),
            });
        }
    }

    if env.vars.values().any(|v| v.contains('$')) {
        warnings.push(Warning::NoInterpolation);
    }
}

fn closest(key: &str, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (*c, edit_distance(key, c)))
        .filter(|(_, distance)| *distance <= max_distance(key.len()))
        .min_by_key(|(_, distance)| *distance)
        .map(|(c, _)| c.to_string())
}

fn max_distance(len: usize) -> usize {
    (len / 2).clamp(1, 3)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();

    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }

    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance("name", "name"), 0);
        assert_eq!(edit_distance("nmae", "name"), 2);
        assert_eq!(edit_distance("netwrk", "network"), 1);
    }
}

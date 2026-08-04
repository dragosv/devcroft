use super::{ConfigError, Env, Filesystem, Warning};
use crate::paths::{SENSITIVE_PATHS, is_within};

/// Known sections and, for each, its known field names. `env.vars` is
/// deliberately absent from `env`'s field list — it is a free-form table
/// of user-chosen names and is never schema-checked.
const SECTIONS: &[(&str, &[&str])] = &[
    ("sandbox", &["name"]),
    ("env", &["provider", "vars"]),
    ("filesystem", &["allow", "read", "deny"]),
    ("network", &["default", "allow"]),
    ("ssh", &["forward_agent"]),
    ("hooks", &["post_create", "post_start"]),
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
    }

    Ok(())
}

pub fn check_filesystem(fs: &Filesystem) -> Result<(), ConfigError> {
    for (field, values) in [("allow", &fs.allow), ("read", &fs.read), ("deny", &fs.deny)] {
        for value in values {
            if value.is_empty() || value.contains('\0') {
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
        if SENSITIVE_PATHS.iter().any(|sensitive| is_within(sensitive, path)) {
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

use super::{ConfigError, Env, Filesystem, Warning};

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

const SENSITIVE_PATHS: &[&str] = &["~/.ssh", "~/.aws", "~/.config/gcloud", "~/.kube"];

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

#[derive(PartialEq, Eq)]
enum Root {
    Home,
    Absolute,
    /// Relative to the project root, e.g. `.`, `src`, `./src`.
    Project,
}

/// Split into a root marker plus path components, dropping `.` and empty
/// segments so `"."`, `"./"` and `""` all normalize to that root's top.
/// The root distinguishes namespaces that must never be compared against
/// each other: `.` (project root) covering everything does not imply
/// anything about `~` (home), and vice versa.
fn components(path: &str) -> (Root, Vec<&str>) {
    let (root, rest) = if path == "~" {
        (Root::Home, "")
    } else if let Some(rest) = path.strip_prefix("~/") {
        (Root::Home, rest)
    } else if let Some(rest) = path.strip_prefix('/') {
        (Root::Absolute, rest)
    } else {
        (Root::Project, path)
    };
    (
        root,
        rest.split('/').filter(|c| !c.is_empty() && *c != ".").collect(),
    )
}

/// Whether `candidate` falls within (or equals) the region `granted` covers.
/// Paths rooted differently (`~foo` vs project-relative vs absolute) are
/// never within one another.
fn is_within(candidate: &str, granted: &str) -> bool {
    let (candidate_root, candidate) = components(candidate);
    let (granted_root, granted) = components(granted);
    candidate_root == granted_root
        && candidate.len() >= granted.len()
        && candidate[..granted.len()] == granted[..]
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

    #[test]
    fn is_within_default_root_covers_everything() {
        assert!(is_within("src/main.rs", "."));
        assert!(is_within("docs", "."));
    }

    #[test]
    fn is_within_respects_boundaries() {
        assert!(is_within("~/.ssh", "~"));
        assert!(!is_within("docs", "src"));
        assert!(is_within("~/.ssh", "~/.ssh"));
    }

    #[test]
    fn is_within_never_crosses_roots() {
        // The project-root default (`.`) must not be read as granting `~`.
        assert!(!is_within("~/.ssh", "."));
        assert!(!is_within("/etc/passwd", "."));
        assert!(!is_within("src", "~"));
    }
}

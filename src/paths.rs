//! Path containment semantics shared by config validation (warn on a
//! sensitive grant) and the policy compiler (baseline-deny a sensitive
//! path unless the manifest already granted it).

/// Credential directories devcroft treats as sensitive: warned on when a
/// manifest explicitly grants them, baseline-denied otherwise.
pub(crate) const SENSITIVE_PATHS: &[&str] = &["~/.ssh", "~/.aws", "~/.config/gcloud", "~/.kube"];

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
pub(crate) fn is_within(candidate: &str, granted: &str) -> bool {
    let (candidate_root, candidate) = components(candidate);
    let (granted_root, granted) = components(granted);
    candidate_root == granted_root
        && candidate.len() >= granted.len()
        && candidate[..granted.len()] == granted[..]
}

#[cfg(test)]
mod tests {
    use super::*;

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

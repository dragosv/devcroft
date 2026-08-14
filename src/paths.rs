//! Path containment semantics shared by config validation (warn on a
//! sensitive grant) and the policy compiler (baseline-deny a sensitive
//! path unless the manifest already granted it) — plus ambient-`PATH`
//! binary resolution shared by anything that spawns a bare command name
//! into a `Command` whose own environment is about to be replaced.

use std::path::{Path, PathBuf};

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
        rest.split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .collect(),
    )
}

/// Whether `path` contains a `..` segment. `is_within`'s containment model
/// (and everything built on it: deny-wins-over-allow, sensitive-path
/// warnings, baseline-deny-unless-already-granted) assumes normalized
/// paths — a `..` segment is a backreference `components()` does not
/// resolve, so it silently breaks every one of those checks rather than
/// erroring. It is also how a `filesystem.allow` entry could escape the
/// project root despite the config spec's requirement that relative paths
/// stay relative to it: devcroft passes manifest path strings to `nono`
/// unresolved, `nono wrap` runs with the project root as its cwd, and `..`
/// resolves exactly as a shell would resolve it — confirmed against a real
/// nono profile granting `../../../secretdir`, which read a file outside
/// the project root with no warning anywhere in devcroft's own validation.
pub(crate) fn has_traversal(path: &str) -> bool {
    let (_, components) = components(path);
    components.contains(&"..")
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

/// Resolves `name` to an absolute path by searching *this process's own*
/// ambient `PATH` — wherever this host actually installed it, which has no
/// reason to fall under any fixed/canonical `PATH` list a subprocess's
/// environment might be replaced with (a devcontainer feature, a package
/// manager, a user install — anywhere). Callers that resolve a binary this
/// way and then hand the `Command` an entirely different `PATH` (e.g. a
/// fixed baseline for `flox activate`, or a provider's resolved env diff
/// for the keeper) must do so *before* that replacement:
/// `std::process::Command`'s own bare-name resolution searches whatever
/// `PATH` is configured *on the command* at spawn time (confirmed: with
/// `.env_clear()` + a fixed `PATH` already applied, `Command::new("flox")`
/// fails to find a real `flox` binary living outside that fixed list) — not
/// the parent process's ambient one.
pub(crate) fn resolve_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        is_executable_file(&candidate).then_some(candidate)
    })
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
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

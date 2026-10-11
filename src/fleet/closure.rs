//! An agent's view of the Nix store, narrowed to its own closure
//! (`workspace-isolation`: "the closure for closure-tier providers").
//!
//! Providers grant the store root (`provider::capture::store_grants`), so
//! with a mount view an agent saw all of `/nix/store`: every other project's
//! toolchain on the host, 6,886 paths here, for a closure of 80
//! (`devbox-citytime-sample`) or 91 (`flox-services-sample`). Visibility,
//! not confidentiality: the store is world-readable on the host anyway. But
//! the spec says the closure, and a view that shows only what the agent
//! runs is the one where "the agent sees exactly its grants" means
//! something.
//!
//! **The roots are what the environment names, directly or through a
//! symlink.** Most of a provider's store paths reach the environment through
//! links in the workspace (devbox's `.devbox/nix/profile/default`, flox's
//! `.flox/run/…`), not as literal `/nix/store` strings, so each absolute path
//! in a value is canonicalized and kept if it lands in the store. Then
//! `nix-store -qR` gives their requisites: everything a binary in them can
//! load. Host-side, in the trusted phase, about 50 ms.
//!
//! Fleet only, for now: `up`'s single sandbox keeps the store root, and its
//! `policy --render` with it.

use std::collections::BTreeSet;
use std::path::Path;

const STORE: &str = "/nix/store";

/// The store path `path` lies in (`/nix/store/<hash>-<name>`), if any.
fn store_root_of(path: &Path) -> Option<String> {
    let rest = path.strip_prefix(STORE).ok()?;
    let top = rest.components().next()?;
    Some(format!("{STORE}/{}", top.as_os_str().to_string_lossy()))
}

/// Every store path the environment names: literally, or as the target of a
/// symlink one of its absolute paths passes through.
pub fn roots<'a>(values: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    for value in values {
        for token in value.split(|c: char| c == ':' || c.is_whitespace() || c == '"') {
            if !token.starts_with('/') {
                continue;
            }
            let path = Path::new(token);
            if let Some(root) = store_root_of(path) {
                roots.insert(root);
            } else if let Some(root) = path.canonicalize().ok().and_then(|p| store_root_of(&p)) {
                roots.insert(root);
            }
        }
    }
    roots
}

/// `grants` with the store root replaced by the closure of `roots`, or why
/// that could not be computed. Grants that are not the store root are kept
/// as they are; with no store-root grant there is nothing to narrow.
pub fn narrow(grants: &[String], roots: &BTreeSet<String>) -> Result<Vec<String>, String> {
    if !grants.iter().any(|g| g == STORE) || roots.is_empty() {
        return Ok(grants.to_vec());
    }
    let nix_store = crate::paths::resolve_on_path("nix-store").ok_or("no `nix-store` on PATH")?;
    let out = std::process::Command::new(nix_store)
        .arg("--query")
        .arg("--requisites")
        .args(roots)
        .output()
        .map_err(|e| format!("running nix-store: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "nix-store --query --requisites: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let closure: BTreeSet<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with(STORE))
        .map(str::to_owned)
        .collect();
    Ok(grants
        .iter()
        .filter(|g| *g != STORE)
        .cloned()
        .chain(closure)
        .collect())
}

/// A path's store root, for a caller holding one path (the resolved shell).
pub fn root_of(path: &Path) -> Option<String> {
    store_root_of(path).or_else(|| path.canonicalize().ok().and_then(|p| store_root_of(&p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_store_paths_named_directly_or_through_a_link() {
        let dir = std::env::temp_dir().join(format!("devcroft-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A link into the store, as a provider's profile link is. Any existing
        // store path does; the test only needs `canonicalize` to land there.
        let some_store_path = std::fs::read_dir(STORE)
            .ok()
            .and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path())));
        let env = match &some_store_path {
            Some(target) => {
                std::os::unix::fs::symlink(target, dir.join("profile")).unwrap();
                vec![
                    format!(
                        "/nix/store/aaaa-direct/bin:{}/profile:/usr/bin",
                        dir.display()
                    ),
                    "plain words /etc/passwd".to_owned(),
                ]
            }
            None => vec!["/nix/store/aaaa-direct/bin:/usr/bin".to_owned()],
        };
        let found = roots(&env);
        assert!(found.contains("/nix/store/aaaa-direct"), "{found:?}");
        if let Some(target) = some_store_path {
            assert!(
                found.contains(&store_root_of(&target).unwrap()),
                "{found:?}"
            );
        }
        assert!(
            found.iter().all(|r| r.starts_with("/nix/store/")),
            "{found:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grants_without_the_store_root_are_left_alone() {
        let grants = vec!["/usr".to_owned()];
        let roots = BTreeSet::from(["/nix/store/aaaa-x".to_owned()]);
        assert_eq!(narrow(&grants, &roots).unwrap(), grants);
    }
}

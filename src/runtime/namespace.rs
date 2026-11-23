//! The absolute-path namespace (`add-own-userland` D4, rule B): one target
//! per base name, computed once at `up`, so `/bin/<name>` cannot disagree
//! with `<name>`. Linux mounts the directory into the view; macOS redirects
//! execs into it (`super::redirect`).

use std::path::{Path, PathBuf};

use super::BASE_NAMES;

/// The namespace directory's name under a sandbox's artifact directory.
pub const DIR: &str = "ns";

/// The target of every base name that has one. `sh` is the resolved shell;
/// any other name is its first executable hit on the sandbox's `PATH` that
/// lies inside a grant. A runtime hit stays the runtime's link, never the
/// binary it points at: the link's name is what selects the utility.
pub fn compute(
    path: &str,
    shell: &Path,
    runtime_dir: Option<&Path>,
    grants: &[PathBuf],
) -> Vec<(String, PathBuf)> {
    let target = |name: &str| -> Option<PathBuf> {
        if name == "sh" {
            return Some(shell.to_path_buf());
        }
        path.split(':').filter(|d| !d.is_empty()).find_map(|dir| {
            let candidate = Path::new(dir).join(name);
            if runtime_dir == Some(Path::new(dir)) {
                return candidate.symlink_metadata().is_ok().then_some(candidate);
            }
            let real = candidate.canonicalize().ok()?;
            (is_executable_file(&real) && grants.iter().any(|g| real.starts_with(g)))
                .then_some(real)
        })
    };
    BASE_NAMES
        .iter()
        .filter_map(|n| Some(((*n).to_owned(), target(n)?)))
        .collect()
}

fn is_executable_file(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_first_then_runtime_and_never_outside_a_grant() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let t = std::env::temp_dir().join(format!("dc-ns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        for d in ["env", "host", "rt"] {
            std::fs::create_dir_all(t.join(d)).unwrap();
        }
        let t = t.canonicalize().unwrap();
        for f in ["env/ls", "host/ls", "host/awk", "host/cat"] {
            std::fs::write(t.join(f), "x").unwrap();
            std::fs::set_permissions(t.join(f), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // A granted directory's link out of the grant does not count.
        symlink(t.join("host/awk"), t.join("env/awk")).unwrap();
        for n in ["ls", "cat"] {
            symlink(t.join("host/cat"), t.join("rt").join(n)).unwrap();
        }
        let dir = |d: &str| t.join(d).to_string_lossy().into_owned();
        let path = [dir("env"), dir("host"), dir("rt")].join(":");
        let got: std::collections::BTreeMap<_, _> = compute(
            &path,
            Path::new("/s/sh"),
            Some(&t.join("rt")),
            &[t.join("env"), t.join("rt")],
        )
        .into_iter()
        .collect();
        assert_eq!(got["sh"], Path::new("/s/sh"));
        assert_eq!(got["ls"], t.join("env/ls"));
        assert_eq!(got["cat"], t.join("rt/cat"));
        assert!(!got.contains_key("awk"));
        let _ = std::fs::remove_dir_all(&t);
    }
}

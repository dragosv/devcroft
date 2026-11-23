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

/// What a Linux view holds for the runtime, built from [`compute`]'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeView {
    /// Links at `/bin/<name>` and `/usr/bin/<name>`, and their targets.
    pub entries: Vec<(PathBuf, PathBuf)>,
    /// Directories remounted read-only over whatever grant covers them.
    pub read_only: Vec<PathBuf>,
}

/// Where a view puts each target of `namespace`: `/bin/<name>` and
/// `/usr/bin/<name>`, once where the host's `/bin` is `/usr/bin`. A place a
/// grant already covers is left to the grant: the manifest asked for the
/// host's directory, its mount is the host's own, and the record then says
/// `host:`.
pub fn view_entries(
    namespace: &[(String, PathBuf)],
    grants: &[PathBuf],
) -> Vec<(PathBuf, PathBuf)> {
    let mut out: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (name, target) in namespace {
        for dir in ["/bin", "/usr/bin"] {
            let at = super::provenance::view_location(&Path::new(dir).join(name));
            if grants.iter().any(|g| at.starts_with(g)) || out.iter().any(|(p, _)| *p == at) {
                continue;
            }
            out.push((at, target.clone()));
        }
    }
    out
}

impl RuntimeView {
    /// `entries` as the view writes them. A runtime entry links devcroft's
    /// binary, not the runtime directory's link to it: the entry's own name
    /// selects the utility, and the directory is in the project, where a
    /// parent of it can be moved from inside.
    pub fn new(
        entries: &[(PathBuf, PathBuf)],
        runtime_dir: &Path,
        exe: &Path,
        read_only: Vec<PathBuf>,
    ) -> Self {
        let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
        Self {
            entries: entries
                .iter()
                .map(|(at, target)| {
                    let runtime = target.parent() == Some(runtime_dir);
                    (
                        at.clone(),
                        if runtime { exe.clone() } else { target.clone() },
                    )
                })
                .collect(),
            read_only,
        }
    }

    /// Every path as `f` rewrites it (a fleet agent's `/workspace`).
    pub fn map(&self, f: impl Fn(&Path) -> PathBuf) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .map(|(at, target)| (at.clone(), f(target)))
                .collect(),
            read_only: self.read_only.iter().map(|p| f(p)).collect(),
        }
    }
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

    #[test]
    fn a_granted_directory_keeps_its_places_and_a_runtime_entry_names_the_binary() {
        let ns = [
            ("sh".to_owned(), PathBuf::from("/s/sh")),
            ("ls".to_owned(), PathBuf::from("/p/rt/ls")),
        ];
        assert_eq!(
            view_entries(&ns, &[PathBuf::from("/usr"), PathBuf::from("/bin")]),
            []
        );
        let entries = view_entries(
            &ns,
            &[PathBuf::from("/usr/lib"), PathBuf::from("/usr/bin/dash")],
        );
        let view = RuntimeView::new(
            &entries,
            Path::new("/p/rt"),
            Path::new("/no/devcroft"),
            vec![],
        );
        let target = |name: &str| {
            view.entries
                .iter()
                .find(|(at, _)| at.ends_with(format!("bin/{name}")))
                .map(|(_, t)| t.clone())
        };
        assert_eq!(target("sh"), Some("/s/sh".into()));
        assert_eq!(target("ls"), Some("/no/devcroft".into()));
    }
}

//! The base runtime namespace: the shell and userland a sandbox can run,
//! by name and by absolute path (`add-runtime-provenance`).
//!
//! devcroft does not ship a runtime of its own yet. What this module owns
//! today is the **definition** of the namespace — which names count as
//! shell and userland, and which absolute paths each is looked up at — and
//! the **record** of where each one resolves in a given sandbox
//! ([`provenance`]). The record is what makes the owner's invariant
//! testable: every shell or utility a sandbox can execute comes from a
//! pinned source (the provider's lock, or later a devcroft runtime the
//! project pins), never from the host, except named exceptions.
//!
//! `docs/base-runtime.md` publishes the same list.

pub mod applets;
pub mod layer;
pub mod provenance;

/// The runtime generation this binary embeds (`add-own-userland` D2).
///
/// It identifies the runtime's *behaviour* — the applet inventory and
/// its upstream version ([`applets::VERSION`]), the absolute-path mapping
/// rules, and anything later added to the runtime — not the devcroft
/// release. A project pins it with `[runtime] generation`, and a binary
/// whose generation differs refuses that project at `up`.
pub const GENERATION: u32 = 1;

/// The runtime directory's name under a sandbox's artifact directory
/// (`<project>/.devcroft/<name>/runtime/`).
pub const DIR: &str = "runtime";

/// Write the runtime directory: one link per applet, named after it, to
/// `exe` (devcroft's own binary), which dispatches on that name
/// ([`applets::dispatch_argv0`]). Idempotent: a correct link is left alone,
/// a wrong one replaced, and an entry that is no applet of this generation
/// removed, so the directory always holds exactly this generation.
///
/// The links name `exe` directly rather than the `EXE_LINK` beside the
/// directory: that link sits in the artifact directory, which a view
/// contains only when the manifest grants the project root, and the
/// runtime must not depend on that grant.
pub fn materialize(dir: &std::path::Path, exe: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().and_then(applets::find).is_none() {
            std::fs::remove_file(entry.path())?;
        }
    }
    for applet in applets::INVENTORY {
        let link = dir.join(applet.name);
        match std::fs::read_link(&link) {
            Ok(target) if target == exe => continue,
            Ok(_) => std::fs::remove_file(&link)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // Not a link (a file someone put there): replaced.
            Err(_) => std::fs::remove_file(&link)?,
        }
        std::os::unix::fs::symlink(exe, &link)?;
    }
    Ok(())
}

/// Revision of [`BASE_NAMES`]. Bumped whenever the list changes, so a
/// record written under an older list is recognisable as such.
pub const BASE_NAMES_REVISION: u32 = 1;

/// The base names: the POSIX shell, the coreutils floor measured for
/// `add-own-userland`, and the userland families an environment commonly
/// supplies and scripts call by absolute path.
///
/// Deliberately about *userland*, not about what a runtime ships: a name a
/// future devcroft runtime lacks still belongs here when environments
/// supply it, because a script calling `/bin/awk` is asking the same
/// question either way.
pub const BASE_NAMES: &[&str] = &[
    "sh", "env", // the two every script needs by absolute path
    // coreutils floor
    "ls", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "ln", "touch", "head", "tail", "wc", "sort",
    "uniq", "cut", "tr", "echo", "printf", "test", "pwd", "true", "false", "seq", "basename",
    "dirname", "readlink", "realpath", "stat", "chmod", "mktemp", "tee", "date", "id", "uname",
    "hostname", "yes", "sleep", "nohup",
    // userland families environments commonly supply
    "grep", "sed", "awk", "find", "xargs", "diff", "tar", "gzip",
];

/// The absolute paths `name` is looked up at, in the order a script is
/// likely to use them.
pub fn absolute_paths(name: &str) -> Vec<String> {
    match name {
        "sh" => vec!["/bin/sh".to_owned()],
        "env" => vec!["/usr/bin/env".to_owned()],
        other => vec![format!("/bin/{other}"), format!("/usr/bin/{other}")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialize_writes_exactly_this_generations_links() {
        let dir = std::env::temp_dir().join(format!("dc-rtdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stale"), "").unwrap();
        std::os::unix::fs::symlink("/elsewhere", dir.join("ls")).unwrap();
        let exe = std::path::Path::new("/opt/devcroft/bin/devcroft");
        materialize(&dir, exe).unwrap();
        materialize(&dir, exe).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let mut expected: Vec<String> = applets::INVENTORY
            .iter()
            .map(|a| a.name.to_owned())
            .collect();
        expected.sort();
        assert_eq!(names, expected);
        assert_eq!(std::fs::read_link(dir.join("ls")).unwrap(), exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base_names_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for n in BASE_NAMES {
            assert!(seen.insert(*n), "{n} listed twice");
        }
    }

    #[test]
    fn sh_and_env_have_their_conventional_paths() {
        assert_eq!(absolute_paths("sh"), ["/bin/sh"]);
        assert_eq!(absolute_paths("env"), ["/usr/bin/env"]);
        assert_eq!(absolute_paths("ls"), ["/bin/ls", "/usr/bin/ls"]);
    }
}

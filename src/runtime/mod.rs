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

pub mod provenance;

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

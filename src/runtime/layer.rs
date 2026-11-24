//! The common runtime layer (`add-own-userland` D5): applied after provider
//! resolution, for every provider, so no provider supplies a shell of the
//! host's. It decides one thing, the shell: the environment's own where it
//! has one, otherwise the runtime's (`super::sh`). The host's is never
//! granted (`embed-brush-runtime`).

use std::path::Path;

use crate::shell::ResolvedShell;

/// The shell a sandbox runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellChoice {
    pub shell: ResolvedShell,
    /// Whether it is the runtime's, the environment having none.
    pub runtime: bool,
}

/// The shell for an environment: its own ([`crate::shell::resolve`]), or the
/// runtime's link under `artifact_dir`. The link and never its target, whose
/// name would not reach the shell; the runtime directory's grant covers it.
pub fn resolve_shell(
    env: &std::collections::BTreeMap<String, String>,
    grants: &[String],
    artifact_dir: &Path,
) -> ShellChoice {
    match crate::shell::resolve(env, grants) {
        Some(shell) => ShellChoice {
            shell,
            runtime: false,
        },
        None => ShellChoice {
            shell: ResolvedShell {
                path: artifact_dir.join(super::DIR).join(super::sh::NAME),
                grant: None,
            },
            runtime: true,
        },
    }
}

/// What a sandbox with the runtime's shell is told, at `up` and by `status`.
pub fn runtime_shell_notice() -> String {
    format!(
        "this environment has no shell, so `sh` is devcroft's own ({} {}): signal traps, `$!`, \
         `wait PID`, an `IFS` other than the default and `printf` with unused arguments do not \
         work in it as in another shell \
         (docs/known-gaps.md); add a shell to the environment to use that instead",
        super::sh::UPSTREAM,
        super::sh::VERSION
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn an_environment_without_a_shell_gets_the_runtimes() {
        let choice = resolve_shell(&BTreeMap::new(), &[], Path::new("/p/.devcroft/n"));
        assert!(choice.runtime);
        assert_eq!(choice.shell.path, Path::new("/p/.devcroft/n/runtime/sh"));
        assert_eq!(choice.shell.grant, None);
    }

    #[test]
    fn an_environment_with_a_granted_shell_keeps_it() {
        let dir = std::env::temp_dir().join(format!("dc-layer-own-{}", std::process::id()));
        let bin = dir.join("env/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let sh = bin.join("sh");
        std::fs::write(&sh, "#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = BTreeMap::from([("PATH".to_owned(), bin.to_string_lossy().into_owned())]);
        let grants = [dir.join("env").to_string_lossy().into_owned()];
        let choice = resolve_shell(&env, &grants, &dir);
        assert!(!choice.runtime);
        assert_eq!(choice.shell.path, sh.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

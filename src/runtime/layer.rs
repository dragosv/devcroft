//! The common runtime layer (`add-own-userland` D5): applied after provider
//! resolution, for every provider, so no provider supplies a host shell of
//! its own.
//!
//! Today it decides one thing, the shell. A provider reports what its
//! environment has; when that holds no POSIX shell, this layer supplies the
//! host's, until devcroft's runtime carries a shell of its own
//! (`embed-brush-runtime`). The host shell is then:
//!
//! - reached through a link named `sh` in a directory of its own under the
//!   sandbox's artifact directory, so the sandbox `PATH` gains exactly `sh`
//!   and never the host directory the shell sits in (mise used to put the
//!   host's `/usr/bin` on `PATH` this way);
//! - granted with origin `host-shell`: on Linux as the one file and the C
//!   library files it loads, read from its headers; on macOS as `/bin` and
//!   `/var/select`, the grants mise and pixi measured (Apple's `/bin/sh`
//!   re-executes the shell `/var/select/sh` names, and its libraries live in
//!   the dyld cache the baseline already exposes);
//! - recorded `host:` by the base-runtime record, which is what warns
//!   about it at `up`.
//!
//! mise and pixi each carried a private copy of this fallback; they now
//! report "no shell" and this layer answers, so a third provider cannot
//! grow a fourth copy.

use std::path::Path;

use crate::shell::ResolvedShell;

/// The directory, under a sandbox's artifact directory, that holds the
/// host shell's `sh` link when the layer supplies it.
pub const HOST_SHELL_DIR: &str = "host-shell";

/// The shell a sandbox runs, and what the layer had to grant for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellChoice {
    pub shell: ResolvedShell,
    /// Grants the host shell needs, origin `host-shell`; empty when the
    /// environment supplied the shell.
    pub host_grants: Vec<String>,
}

impl ShellChoice {
    /// Whether the shell is the host's, supplied by this layer.
    pub fn is_host(&self) -> bool {
        !self.host_grants.is_empty()
    }
}

/// The shell for an environment: its own when it has one
/// ([`crate::shell::resolve`]), otherwise the host's, linked as `sh` inside
/// `artifact_dir` and granted. `Err` names what is missing.
pub fn resolve_shell(
    env: &std::collections::BTreeMap<String, String>,
    grants: &[String],
    artifact_dir: &Path,
) -> Result<ShellChoice, String> {
    if let Some(shell) = crate::shell::resolve(env, grants) {
        return Ok(ShellChoice {
            shell,
            host_grants: Vec::new(),
        });
    }
    host_shell(Path::new("/bin/sh"), artifact_dir)
}

/// The host shell at `host` (`/bin/sh` outside tests), linked as `sh` under
/// `artifact_dir`, with the grants it needs.
fn host_shell(host: &Path, artifact_dir: &Path) -> Result<ShellChoice, String> {
    let real = host
        .canonicalize()
        .map_err(|e| format!("host POSIX shell {}: {e}", host.display()))?;
    let dir = artifact_dir.join(HOST_SHELL_DIR);
    super::owned_dir::sync_links(
        artifact_dir,
        HOST_SHELL_DIR,
        &[(crate::shell::SHELL_NAME.to_owned(), real.clone())],
    )
    .map_err(|e| e.to_string())?;
    let link = dir.join(crate::shell::SHELL_NAME);
    let mut host_grants = vec![dir.to_string_lossy().into_owned()];
    host_grants.extend(platform_grants(&real)?);
    host_grants.sort();
    host_grants.dedup();
    Ok(ShellChoice {
        shell: ResolvedShell {
            // The link, not its target: its directory is what goes on
            // `PATH`, and it holds `sh` and nothing else.
            path: link,
            grant: None,
        },
        host_grants,
    })
}

#[cfg(target_os = "macos")]
fn platform_grants(_shell: &Path) -> Result<Vec<String>, String> {
    Ok(vec!["/bin".to_owned(), "/var/select".to_owned()])
}

/// The shell file and the C library files it loads, transitively. Only the
/// glibc set mise and pixi qualified is followed; anything else fails
/// closed, naming the library, rather than starting a sandbox whose shell
/// cannot load.
#[cfg(not(target_os = "macos"))]
fn platform_grants(shell: &Path) -> Result<Vec<String>, String> {
    const GLIBC: &[&str] = &[
        "ld-linux-aarch64.so.1",
        "ld-linux-x86-64.so.2",
        "libc.so.6",
        "libm.so.6",
        "libdl.so.2",
        "libpthread.so.0",
        "libutil.so.1",
        "librt.so.1",
        "libresolv.so.2",
        // bash, where it is the host's `sh`.
        "libtinfo.so.6",
    ];
    const DIRS: &[&str] = &[
        "/lib/aarch64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib/x86_64-linux-gnu",
        "/usr/lib/x86_64-linux-gnu",
        "/lib64",
        "/lib",
        "/usr/lib",
    ];
    let mut grants = vec![shell.to_string_lossy().into_owned()];
    let mut queue = vec![shell.to_path_buf()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(binary) = queue.pop() {
        if !seen.insert(binary.clone()) {
            continue;
        }
        let needed = crate::provider::artifacts::header_dependencies("host-shell", &binary)
            .map_err(|e| e.to_string())?;
        for needed in needed {
            let base = Path::new(&needed)
                .file_name()
                .and_then(|b| b.to_str())
                .unwrap_or_default()
                .to_owned();
            if !GLIBC.contains(&base.as_str()) {
                return Err(format!(
                    "the host shell {} needs {needed}, outside the qualified C library set",
                    shell.display()
                ));
            }
            let candidate: Option<std::path::PathBuf> = if needed.starts_with('/') {
                Some(PathBuf::from(&needed))
            } else {
                DIRS.iter()
                    .map(|d| Path::new(d).join(&base))
                    .find(|p| p.is_file())
            };
            let real = candidate
                .and_then(|c| c.canonicalize().ok())
                .ok_or_else(|| {
                    format!(
                        "the host shell {} needs {needed}, not found",
                        shell.display()
                    )
                })?;
            grants.push(real.to_string_lossy().into_owned());
            queue.push(real);
        }
    }
    Ok(grants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dc-layer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_environment_without_a_shell_gets_the_hosts_as_one_named_link() {
        let dir = scratch("host");
        let choice = resolve_shell(&BTreeMap::new(), &[], &dir).unwrap();
        assert!(choice.is_host());
        let link = dir.join(HOST_SHELL_DIR).join("sh");
        assert_eq!(choice.shell.path, link);
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            Path::new("/bin/sh").canonicalize().unwrap()
        );
        // The link directory holds `sh` alone, so putting it on `PATH`
        // adds no other host name.
        let names: Vec<_> = std::fs::read_dir(dir.join(HOST_SHELL_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["sh"]);
        assert!(
            choice
                .host_grants
                .contains(&dir.join(HOST_SHELL_DIR).to_string_lossy().into_owned())
        );
        // Idempotent.
        assert_eq!(resolve_shell(&BTreeMap::new(), &[], &dir).unwrap(), choice);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_environment_with_a_granted_shell_keeps_it_and_grants_nothing() {
        let dir = scratch("own");
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
        let choice = resolve_shell(&env, &grants, &dir).unwrap();
        assert!(!choice.is_host());
        assert_eq!(choice.shell.path, sh.canonicalize().unwrap());
        assert!(!dir.join(HOST_SHELL_DIR).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! The macOS exec-redirect library (`add-macos-exec-redirect`), embedded at
//! build time from `redirect.c` (see `build.rs`).
//!
//! Loaded into sandbox processes through `DYLD_INSERT_LIBRARIES`, it runs a
//! base-name path (`/bin/<name>`, `/usr/bin/<name>`, `/bin/sh`,
//! `/usr/bin/env`) through the sandbox's namespace directory, named by
//! [`NS_VAR`]. It holds no policy of its own: what an entry points at is
//! decided once at `up`, for both platforms.

/// The library's bytes; empty on every target but macOS.
pub const LIBRARY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libdevcroft-redirect.dylib"));

/// The library's file name, beside the runtime links.
pub const FILE_NAME: &str = "libdevcroft-redirect.dylib";

/// The variable naming the namespace directory the library maps into.
pub const NS_VAR: &str = "DEVCROFT_NS";

/// The variable dyld loads the library from.
pub const INSERT_VAR: &str = "DYLD_INSERT_LIBRARIES";

/// Whether this build carries the library.
pub fn available() -> bool {
    !LIBRARY.is_empty()
}

/// The library's directory under a sandbox's artifact directory.
pub const DIR: &str = "redirect";

/// The keeper's own names for the library and the namespace. Never
/// [`INSERT_VAR`] on the keeper itself: dyld would load the library before
/// the keeper restricts itself, and the file is where the sandbox can write.
/// The keeper turns these into [`INSERT_VAR`] and [`NS_VAR`] for its
/// children, which start inside the boundary ([`child_vars`]).
pub const KEEPER_LIB_VAR: &str = "DEVCROFT_REDIRECT_LIB";
pub const KEEPER_NS_VAR: &str = "DEVCROFT_REDIRECT_NS";
pub const KEEPER_TEMP_VAR: &str = "DEVCROFT_REDIRECT_TEMP";
pub const KEEPER_TEMP_GRANTS: &str = "DEVCROFT_REDIRECT_TEMP_GRANTS";
pub const KEEPER_USER_VAR: &str = "DEVCROFT_REDIRECT_USER";
pub const KEEPER_EXEC_LOG_VAR: &str = "DEVCROFT_REDIRECT_EXEC_LOG";

/// Set on the host when running `up` to have the library log every exec
/// into the sandbox's `run/` directory ([`EXEC_LOG`]). A measurement aid,
/// for building option corpora (`add-own-userland` 5.1), not a feature: it
/// changes no policy, since `run/` is already writable from inside.
pub const LOG_EXECS_VAR: &str = "DEVCROFT_LOG_EXECS";

/// The exec log's file name under `run/`.
pub const EXEC_LOG: &str = "exec.log";

const TEMP_VAR: &str = "DEVCROFT_TEMP";
const TEMP_GRANTS: &str = "DEVCROFT_TEMP_GRANTS";

/// Write the library beside the runtime links; returns its path.
pub fn materialize(artifact_dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    super::owned_dir::OwnedDir::open(artifact_dir, DIR)?.write(FILE_NAME, LIBRARY)?;
    Ok(artifact_dir.join(DIR).join(FILE_NAME))
}

/// The variables a sandbox process starts with, from the keeper's own; `own`
/// is the insert list the process would otherwise have, which is kept.
pub fn child_vars(own: Option<std::ffi::OsString>) -> Vec<(&'static str, std::ffi::OsString)> {
    match (
        std::env::var_os(KEEPER_LIB_VAR),
        std::env::var_os(KEEPER_NS_VAR),
    ) {
        (Some(lib), Some(ns)) => {
            let mut vars = vars_for(lib, ns, own);
            for (source, target) in [
                (KEEPER_TEMP_VAR, TEMP_VAR),
                (KEEPER_TEMP_GRANTS, TEMP_GRANTS),
                (KEEPER_USER_VAR, "DEVCROFT_ID_USER"),
                ("HOME", "DEVCROFT_ID_HOME"),
                ("DEVCROFT_SHELL", "DEVCROFT_ID_SHELL"),
                (KEEPER_EXEC_LOG_VAR, "DEVCROFT_EXEC_LOG"),
            ] {
                if let Some(value) = std::env::var_os(source) {
                    vars.push((target, value));
                }
            }
            vars
        }
        _ => Vec::new(),
    }
}

/// Path components under macOS's physical temp root that already have a
/// grant. Lengths are byte counts, so path names can contain separators.
pub fn temp_grants(grants: &[crate::policy::ResolvedGrant]) -> String {
    use std::fmt::Write;
    let host = std::path::Path::new("/private/tmp");
    let mut out = String::new();
    for g in grants {
        let path = if host.starts_with(&g.path) {
            host
        } else if g.path.starts_with(host) {
            &g.path
        } else {
            continue;
        };
        let path = path.to_string_lossy();
        write!(out, "{}:{path}", path.len()).unwrap();
    }
    out
}

/// The same absolute-path mapping used by sandbox children, for programs
/// and working directories opened directly by the keeper.
pub fn temp_path(path: &std::path::Path) -> std::path::PathBuf {
    let Some(temp) = std::env::var_os(KEEPER_TEMP_VAR) else {
        return path.to_owned();
    };
    map_temp(
        path,
        std::path::Path::new(&temp),
        &std::env::var(KEEPER_TEMP_GRANTS).unwrap_or_default(),
    )
}

fn map_temp(path: &std::path::Path, temp: &std::path::Path, grants: &str) -> std::path::PathBuf {
    use std::path::{Component, Path, PathBuf};
    if !path.is_absolute() {
        return path.to_owned();
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            normalized.pop();
        } else {
            normalized.push(component);
        }
    }
    let host = Path::new("/private/tmp");
    let Ok(rest) = normalized
        .strip_prefix("/tmp")
        .or_else(|_| normalized.strip_prefix(host))
    else {
        return path.to_owned();
    };
    let canonical = if rest.as_os_str().is_empty() {
        host.to_owned()
    } else {
        host.join(rest)
    };
    let mut remaining = grants;
    while let Some((len, tail)) = remaining.split_once(':') {
        let Some((grant, tail)) = len.parse().ok().and_then(|len| tail.split_at_checked(len))
        else {
            break;
        };
        if canonical.starts_with(grant) {
            return if grant == "/private/tmp" {
                path.to_owned()
            } else {
                canonical
            };
        }
        remaining = tail;
    }
    if rest.as_os_str().is_empty() {
        temp.to_owned()
    } else {
        temp.join(rest)
    }
}

pub(crate) fn temp_path_for_policy(
    path: &std::path::Path,
    policy: &crate::policy::CompiledPolicy,
    project_root: &std::path::Path,
) -> std::path::PathBuf {
    let Some(temp) = policy.local_temp.as_deref() else {
        return path.to_owned();
    };
    let grants = policy.to_capability_plan().resolved_grants(project_root);
    let encoded = grants.as_deref().map(temp_grants).unwrap_or_default();
    map_temp(path, std::path::Path::new(temp), &encoded)
}

/// What to start in place of a command the redirect would rewrite.
#[derive(Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub program: std::path::PathBuf,
    pub arg0: String,
    /// Before the command's own arguments.
    pub args: Vec<String>,
}

/// The rewrite of the keeper's own spawn of `cmd`, as the library rewrites an
/// exec (`redirect.c`): the keeper never loads it, so a base name's path or a
/// `#!/bin/sh` script it starts itself would reach the host's file and be
/// refused. `None` leaves the command as it is.
pub fn rewrite(cmd: &str, cwd: &std::path::Path, path: Option<&str>) -> Option<Rewrite> {
    let mapped = temp_path(std::path::Path::new(cmd));
    let program = mapped.to_str()?;
    let changed = program != cmd;
    rewrite_in(
        std::path::Path::new(&std::env::var_os(KEEPER_NS_VAR)?),
        program,
        cwd,
        path,
    )
    .or_else(|| {
        changed.then(|| Rewrite {
            program: mapped,
            arg0: cmd.to_owned(),
            args: Vec::new(),
        })
    })
}

fn rewrite_in(
    ns: &std::path::Path,
    cmd: &str,
    cwd: &std::path::Path,
    path: Option<&str>,
) -> Option<Rewrite> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &std::path::Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let entry = |at: &str| {
        let name = at
            .strip_prefix("/bin/")
            .or_else(|| at.strip_prefix("/usr/bin/"))?;
        let entry = ns.join(name);
        (!name.is_empty() && !name.contains('/') && !name.starts_with('.') && executable(&entry))
            .then_some(entry)
    };
    if let Some(program) = entry(cmd) {
        return Some(Rewrite {
            program,
            arg0: cmd.to_owned(),
            args: Vec::new(),
        });
    }
    // The script as its interpreter is given it: the path that was named, or
    // the one the search found.
    let script = if cmd.contains('/') {
        cmd.to_owned()
    } else {
        path?
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| std::path::Path::new(d).join(cmd))
            .find(|p| executable(&cwd.join(p)))?
            .to_str()?
            .to_owned()
    };
    let file = cwd.join(&script);
    if !executable(&file) {
        return None;
    }
    let mut head = Vec::new();
    std::fs::File::open(&file)
        .ok()?
        .take(512)
        .read_to_end(&mut head)
        .ok()?;
    let line = head.strip_prefix(b"#!")?.split(|b| *b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim();
    let (interp, arg) = match line.split_once([' ', '\t']) {
        Some((interp, arg)) => (interp, Some(arg.trim())),
        None => (line, None),
    };
    let program = entry(interp).or_else(|| {
        let at = std::path::Path::new(interp);
        let mapped = temp_path(at);
        (mapped != at).then_some(mapped)
    })?;
    Some(Rewrite {
        program,
        arg0: interp.to_owned(),
        args: arg.map(str::to_owned).into_iter().chain([script]).collect(),
    })
}

/// `existing` GOFLAGS with an `-exec` that gives a program Go starts its
/// redirect back: the official Go is hardened, so dyld drops it there.
/// `None` where there is an `-exec` already, or a path has a quote in it
/// (GOFLAGS has no escape for one).
pub fn go_flags(
    existing: Option<&str>,
    env: &std::path::Path,
    lib: &std::path::Path,
    ns: &std::path::Path,
) -> Option<String> {
    let existing = existing.unwrap_or_default().trim();
    let paths = [env, lib, ns].map(|p| p.to_string_lossy().into_owned());
    if existing.contains("-exec") || paths.iter().any(|p| p.contains(['\'', '"'])) {
        return None;
    }
    let [env, lib, ns] = paths;
    let flag = format!("\"-exec='{env}' '{INSERT_VAR}={lib}' '{NS_VAR}={ns}'\"");
    Some(if existing.is_empty() {
        flag
    } else {
        format!("{existing} {flag}")
    })
}

fn vars_for(
    lib: std::ffi::OsString,
    ns: std::ffi::OsString,
    own: Option<std::ffi::OsString>,
) -> Vec<(&'static str, std::ffi::OsString)> {
    let mut insert = lib.clone();
    for path in own.iter().flat_map(std::env::split_paths) {
        if !path.as_os_str().is_empty() && path.as_os_str() != lib {
            insert.push(":");
            insert.push(path);
        }
    }
    vec![(INSERT_VAR, insert), (NS_VAR, ns)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_mapping_preserves_grants_and_normalizes_both_spellings() {
        use std::path::Path;
        let temp = Path::new("/private/tmp/dc-501/sandbox");
        let grants = ["/private/tmp/project", temp.to_str().unwrap()]
            .map(|p| format!("{}:{p}", p.len()))
            .concat();
        for (source, expected) in [
            ("/tmp", "/private/tmp/dc-501/sandbox"),
            ("/private/tmp/./x", "/private/tmp/dc-501/sandbox/x"),
            ("/tmp/../tmp/x", "/private/tmp/dc-501/sandbox/x"),
            ("/a/../tmp/x", "/private/tmp/dc-501/sandbox/x"),
            ("/tmp/project/x", "/private/tmp/project/x"),
            (
                "/private/tmp/dc-501/sandbox/x",
                "/private/tmp/dc-501/sandbox/x",
            ),
            ("/tmp-other/x", "/tmp-other/x"),
            ("tmp/x", "tmp/x"),
        ] {
            assert_eq!(
                map_temp(Path::new(source), temp, &grants),
                Path::new(expected)
            );
        }
        assert_eq!(
            map_temp(Path::new("/tmp/x"), temp, "12:/private/tmp"),
            Path::new("/tmp/x")
        );
    }

    #[test]
    fn the_library_is_embedded_exactly_on_macos() {
        assert_eq!(available(), cfg!(target_os = "macos"));
        if available() {
            // A 64-bit Mach-O (thin or universal).
            assert!(
                LIBRARY.starts_with(b"\xcf\xfa\xed\xfe")
                    || LIBRARY.starts_with(b"\xca\xfe\xba\xbe"),
                "not a Mach-O"
            );
        }
    }

    #[test]
    fn the_keepers_own_spawns_are_rewritten_as_the_library_would() {
        use std::os::unix::fs::PermissionsExt;
        let t = std::env::temp_dir().join(format!("dc-rewrite-{}", std::process::id()));
        std::fs::create_dir_all(t.join("ns")).unwrap();
        std::fs::create_dir_all(t.join("bin")).unwrap();
        let file = |name: &str, text: &str, mode| {
            std::fs::write(t.join(name), text).unwrap();
            std::fs::set_permissions(t.join(name), std::fs::Permissions::from_mode(mode)).unwrap();
        };
        file("ns/sh", "x", 0o755);
        file("ns/env", "x", 0o755);
        file("ns/date", "x", 0o644);
        file("s", "#!/bin/sh\n", 0o755);
        file("bin/e", "#! /usr/bin/env  -S python3 x\r\n", 0o755);
        file("plain", "#!/bin/sh\n", 0o644);
        file("other", "#!/opt/sh\n", 0o755);
        let bin = t.join("bin").to_str().unwrap().to_owned();
        let go = |cmd: &str| {
            rewrite_in(&t.join("ns"), cmd, &t, Some(&bin)).map(|r| {
                assert!(r.program.starts_with(t.join("ns")));
                (r.arg0, r.args)
            })
        };
        assert_eq!(go("/bin/sh"), Some(("/bin/sh".into(), vec![])));
        assert_eq!(go("./s"), Some(("/bin/sh".into(), vec!["./s".into()])));
        assert_eq!(
            go("e"),
            Some((
                "/usr/bin/env".into(),
                vec!["-S python3 x".into(), format!("{bin}/e")]
            ))
        );
        for cmd in [
            "/bin/date",
            "/bin/cat",
            "/bin/../bin/sh",
            "/usr/bin/a/b",
            "./plain",
            "./other",
            "missing",
            "./missing",
            "sh",
        ] {
            assert_eq!(go(cmd), None, "{cmd}");
        }
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn go_flags_keep_what_is_there_and_never_replace_an_exec() {
        use std::path::Path;
        let flags = |existing, dir: &str| {
            let d = Path::new(dir);
            go_flags(existing, &d.join("env"), &d.join("l"), &d.join("ns"))
        };
        assert_eq!(
            flags(Some("-mod=mod"), "/a b").unwrap(),
            "-mod=mod \"-exec='/a b/env' 'DYLD_INSERT_LIBRARIES=/a b/l' 'DEVCROFT_NS=/a b/ns'\""
        );
        assert!(flags(None, "/a").unwrap().starts_with("\"-exec='/a/env'"));
        assert_eq!(flags(Some("-exec=x"), "/a"), None);
        assert_eq!(flags(None, "/it's"), None);
    }

    #[test]
    fn the_library_comes_first_once_and_keeps_the_processs_own() {
        let insert = |own: Option<&str>| {
            vars_for("/l".into(), "/ns".into(), own.map(Into::into))[0]
                .1
                .clone()
        };
        assert_eq!(insert(None), "/l");
        assert_eq!(insert(Some("")), "/l");
        assert_eq!(insert(Some("/x")), "/l:/x");
        assert_eq!(insert(Some("/x:/l")), "/l:/x");
    }
}

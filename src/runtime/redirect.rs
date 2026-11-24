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
        (Some(lib), Some(ns)) => vars_for(lib, ns, own),
        _ => Vec::new(),
    }
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

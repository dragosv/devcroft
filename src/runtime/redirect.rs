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
    let (Some(lib), Some(ns)) = (
        std::env::var_os(KEEPER_LIB_VAR),
        std::env::var_os(KEEPER_NS_VAR),
    ) else {
        return Vec::new();
    };
    let mut insert = lib.clone();
    if let Some(own) = own.filter(|o| !o.is_empty())
        && !std::env::split_paths(&own).any(|p| p.as_os_str() == lib)
    {
        insert.push(":");
        insert.push(own);
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
}

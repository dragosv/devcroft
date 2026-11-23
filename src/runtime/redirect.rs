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

/// Write the library to `path` unless it already holds these bytes, so a
/// running sandbox never sees it rewritten in place. Written to a temporary
/// name and renamed, since dyld maps the file.
pub fn materialize(path: &std::path::Path) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|b| b == LIBRARY) {
        return Ok(());
    }
    let tmp = path.with_extension("dylib.tmp");
    std::fs::write(&tmp, LIBRARY)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, path)
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

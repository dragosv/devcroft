//! Builds the macOS exec-redirect library (`add-macos-exec-redirect` D4) and
//! leaves it in `OUT_DIR` for `src/runtime/redirect.rs` to embed.
//!
//! `cargo install` installs binaries only, so the library cannot ship as a
//! file of its own: it is compiled from `src/runtime/redirect.c` here, with
//! the C compiler the `cc` crate selects for the target, and written at `up`.
//! The linker ad-hoc signs it, which arm64 requires. Other targets get an
//! empty file and embed nothing.

use std::path::PathBuf;

fn main() {
    let source = "src/runtime/redirect.c";
    println!("cargo:rerun-if-changed={source}");
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let dylib = out.join("libdevcroft-redirect.dylib");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        std::fs::write(&dylib, b"").expect("write the empty placeholder");
        return;
    }
    let compiler = cc::Build::new()
        .cargo_metadata(false)
        .opt_level(2)
        .get_compiler();
    let status = compiler
        .to_command()
        .args([
            "-dynamiclib",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fvisibility=hidden",
            "-install_name",
            "@rpath/libdevcroft-redirect.dylib",
            "-o",
        ])
        .arg(&dylib)
        .arg(source)
        .status()
        .expect("run the C compiler for the exec-redirect library");
    assert!(status.success(), "compiling {source} failed");
}

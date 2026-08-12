// This binary exists to demonstrate a toolchain split, not to do anything
// interesting on its own:
//   - rustup (via rust-toolchain.toml) pins the exact Rust *version*.
//   - flox (via .flox/env/manifest.toml) provides everything rustup itself
//     doesn't: the C toolchain rustc needs to link anything, plus rustup
//     itself, reproducibly and pinned by lockfile.
//   - devcroft (via devcroft.toml) wraps the two in a sandboxed session, so
//     `devcroft exec -- cargo build` runs this exact combination inside a
//     kernel-enforced boundary instead of directly on the host.
fn main() {
    println!(
        "flox-rustup-sample: hello from {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("built with rustc {}", rustc_version());
}

fn rustc_version() -> &'static str {
    env!("RUSTC_VERSION_FOR_SAMPLE")
}

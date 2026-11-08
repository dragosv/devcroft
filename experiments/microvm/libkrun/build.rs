use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let patch = root.join("patches/libkrun-vsock-host-half-close.patch");
    let source = root.join("artifacts/libkrun-source");
    println!("cargo:rerun-if-changed={}", patch.display());
    println!(
        "cargo:rerun-if-changed={}",
        root.join("build_helper.py").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        source.join("Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        source.join(".git/HEAD").display()
    );
    println!("cargo:rerun-if-changed={}", source.join("src").display());
    let verification = Command::new("python3")
        .arg(root.join("build_helper.py"))
        .arg("--prepare-only")
        .output()
        .expect("run source provenance verification");
    assert!(
        verification.status.success(),
        "prepare libkrun with build_helper.py: {}",
        String::from_utf8_lossy(&verification.stderr)
    );
    let hash = format!("{:x}", Sha256::digest(fs::read(patch).unwrap()));
    println!("cargo:rustc-env=DEVCROFT_KRUN_PATCH_SHA256={hash}");
}

use std::process::Command;

const PIN: &str = "351d1a15728e998486232e4815ba075a22e44f56";

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for path in [
        "build_hvi.py",
        "patches/hvi-vsock-fixed-ports.patch",
        "artifacts/hvi-source/src",
        "artifacts/hvi-source/Cargo.toml",
        "artifacts/hvi-source/.git/HEAD",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let verification = Command::new("python3")
        .arg(root.join("build_hvi.py"))
        .arg("--prepare-only")
        .output()
        .expect("source provenance verification");
    assert!(
        verification.status.success(),
        "hvi source verification: {}",
        String::from_utf8_lossy(&verification.stderr)
    );
    let stamp: serde_json::Value =
        serde_json::from_slice(&verification.stdout).expect("source receipt");
    println!("cargo:rustc-env=DEVCROFT_HVI_REVISION={PIN}");
    println!(
        "cargo:rustc-env=DEVCROFT_HVI_PATCH_SHA256={}",
        stamp["patch_sha256"].as_str().unwrap()
    );
}

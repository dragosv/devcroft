use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = Command::new(rustc)
        .arg("--version")
        .output()
        .expect("failed to run rustc --version");
    let version = String::from_utf8_lossy(&output.stdout);
    println!(
        "cargo:rustc-env=RUSTC_VERSION_FOR_SAMPLE={}",
        version.trim()
    );
}

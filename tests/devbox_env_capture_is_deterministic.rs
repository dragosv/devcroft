//! Mirrors `flox_env_capture_is_deterministic.rs`/
//! `nix_env_capture_is_deterministic.rs` for `DevboxProvider`: a decoy
//! `PATH` entry and an arbitrary env var on the invoking shell must not
//! leak into the captured activation diff, since `provider::capture`'s
//! fixed-baseline mechanism (design.md decision 2) is shared by every
//! provider rather than reimplemented per provider.
//!
//! Mutates the test process's own `PATH`, so — same reasoning as the flox
//! and nix versions — this needs to be alone in its own process.

use devcroft::provider::{DevboxProvider, Provider};
use std::process::Command;

fn devbox_available() -> bool {
    Command::new("devbox")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_decoy_path_entry_on_the_invoking_shell_does_not_leak_into_the_activation() {
    if !devbox_available() || Command::new("nix").arg("--version").output().is_err() {
        eprintln!("skipping: devbox or nix not on PATH");
        return;
    }

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-devbox-determinism-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    std::fs::write(
        project_root.join("devbox.json"),
        r#"{"packages": ["ripgrep@latest"]}"#,
    )
    .unwrap();

    let add = Command::new("devbox")
        .arg("install")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !add.status.success() {
        eprintln!(
            "skipping: devbox install failed (likely no network for nixpkgs): {}",
            String::from_utf8_lossy(&add.stderr)
        );
        let _ = std::fs::remove_dir_all(&project_root);
        return;
    }

    // A directory that looks exactly like a real operator's personal tool
    // dir: a marker env var an activation-time leak could plausibly carry,
    // plus a fake binary that must never appear reachable afterward.
    let decoy_dir =
        std::env::temp_dir().join(format!("devcroft-devbox-decoy-{}", std::process::id()));
    std::fs::create_dir_all(&decoy_dir).unwrap();
    std::fs::write(
        decoy_dir.join("not-a-real-tool"),
        "#!/bin/sh\necho leaked\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(decoy_dir.join("not-a-real-tool"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(decoy_dir.join("not-a-real-tool"), perms).unwrap();
    }

    let real_path = std::env::var("PATH").unwrap();
    let contaminated_path = format!("{}:{real_path}", decoy_dir.display());
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("PATH", &contaminated_path);
        std::env::set_var("DEVCROFT_LEAK_MARKER", "should-not-survive");
    }

    let resolution = DevboxProvider.resolve(&project_root).unwrap();

    let path_in_diff = resolution.env.get("PATH").cloned().unwrap_or_default();
    assert!(
        !path_in_diff.contains(&decoy_dir.display().to_string()),
        "the decoy PATH entry from the invoking shell leaked into the \
         activation diff's PATH: {path_in_diff:?}"
    );
    assert!(
        !resolution.env.contains_key("DEVCROFT_LEAK_MARKER"),
        "an arbitrary env var from the invoking shell leaked into the \
         activation diff: {:?}",
        resolution.env
    );
    assert!(
        !resolution.read_only_grants.is_empty(),
        "expected at least a store-root grant"
    );

    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::remove_dir_all(&decoy_dir);
}

//! Mirrors `flox_env_capture_is_deterministic.rs` for `NixProvider`: a
//! decoy `PATH` entry and an arbitrary env var on the invoking shell must
//! not leak into the captured activation diff, since `provider::capture`'s
//! fixed-baseline mechanism (design.md decision 2) is shared by both
//! providers rather than reimplemented per provider.
//!
//! Mutates the test process's own `PATH`, so — same reasoning as the flox
//! version — this needs to be alone in its own process.

use devcroft::provider::{NixProvider, Provider};
use std::process::Command;

/// Same fixture shape as `provider::nix`'s own unit tests: enumerates
/// systems statically rather than reading `builtins.currentSystem`
/// (unavailable under nix's pure evaluation).
const FLAKE_NIX: &str = r#"
{
  description = "devcroft nix determinism fixture";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      shellFor = system: (import nixpkgs { inherit system; }).mkShell {
        DEVCROFT_NIX_MARKER = "real-activation-value";
      };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
"#;

#[test]
fn a_decoy_path_entry_on_the_invoking_shell_does_not_leak_into_the_activation() {
    let flakes_enabled = Command::new("nix")
        .arg("flake")
        .arg("--help")
        .output()
        .is_ok_and(|o| o.status.success());
    if !flakes_enabled {
        eprintln!("skipping: nix not on PATH or flakes not enabled");
        return;
    }

    let project_root =
        std::env::temp_dir().join(format!("devcroft-nix-determinism-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    std::fs::write(project_root.join("flake.nix"), FLAKE_NIX).unwrap();

    let lock = Command::new("nix")
        .arg("flake")
        .arg("lock")
        .arg(&project_root)
        .output()
        .unwrap();
    if !lock.status.success() {
        eprintln!(
            "skipping: nix flake lock failed (likely no network for nixpkgs): {}",
            String::from_utf8_lossy(&lock.stderr)
        );
        let _ = std::fs::remove_dir_all(&project_root);
        return;
    }

    // A directory that looks exactly like a real operator's personal tool
    // dir: a marker env var an activation-time leak could plausibly carry,
    // plus a fake binary that must never appear reachable afterward.
    let decoy_dir = std::env::temp_dir().join(format!("devcroft-nix-decoy-{}", std::process::id()));
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

    let resolution = NixProvider.resolve(&project_root).unwrap();

    assert_eq!(
        resolution
            .env
            .get("DEVCROFT_NIX_MARKER")
            .map(String::as_str),
        Some("real-activation-value"),
        "the real activation value must still be captured despite PATH contamination"
    );

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

    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::remove_dir_all(&decoy_dir);
}

//! End-to-end coverage for the `nix` provider (add-nix-provider task 5),
//! through the real built binary against a real `nix` sandbox —
//! same pattern `cli_lifecycle_and_policy.rs` uses for flox. Skips
//! quietly wherever a flakes-enabled `nix` isn't on PATH,
//! same as every other real-tooling test in this suite.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn run(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(devcroft_bin())
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn nix_available() -> bool {
    Command::new("nix")
        .arg("flake")
        .arg("--help")
        .output()
        .is_ok_and(|o| o.status.success())
        // `nix flake --help` succeeds without a usable store, which is the
        // same class of mistake `doctor`'s own nix check was once bitten
        // by (README, add-nix-provider) — see
        // `provider::host_can_build_nix_closures`.
        && devcroft::provider::host_can_build_nix_closures()
}

/// A minimal, real flake exporting one distinctive env var — same fixture
/// shape `provider::nix`'s own unit tests use, enumerating systems
/// statically rather than reading `builtins.currentSystem` (unavailable
/// under nix's pure evaluation, which this provider never overrides).
const FLAKE_NIX: &str = r#"
{
  description = "devcroft nix e2e fixture";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      shellFor = system: (import nixpkgs { inherit system; }).mkShell {
        DEVCROFT_NIX_E2E = "present";
      };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
"#;

struct Sandbox {
    name: String,
    project_root: PathBuf,
}

impl Sandbox {
    /// `locked`: whether to run `nix flake lock` during setup. Failure
    /// paths (missing-lock tests) need a flake *without* one.
    fn new(tag: &str, locked: bool) -> Option<Self> {
        if !nix_available() {
            eprintln!("skipping: a flakes-enabled nix is not on PATH");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }

        let project_root =
            std::env::temp_dir().join(format!("devcroft-nix-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&project_root);
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::write(project_root.join("flake.nix"), FLAKE_NIX).unwrap();

        if locked {
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
                return None;
            }
        }

        let name = format!("e2enix{tag}{}", std::process::id());
        std::fs::write(
            project_root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"nix\"\n"),
        )
        .unwrap();

        Some(Sandbox { name, project_root })
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        run(&self.project_root, args)
    }

    fn state_root(&self) -> PathBuf {
        devcroft::lifecycle::StatePaths::new(&self.name)
            .unwrap()
            .root
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(self.state_root());
        let _ = std::fs::remove_dir_all(&self.project_root);
    }
}

#[test]
fn up_resolves_the_flake_and_the_dev_shell_is_visible_in_a_session() {
    let Some(sandbox) = Sandbox::new("up", true) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("is started"));

    // The default manifest's network stays deny (spec: "Toolchain works
    // under network deny-all") — materialization already happened
    // host-side at `up`, so no session-time network is needed for this.
    let out = sandbox.run(&["exec", "--", "sh", "-c", "echo $DEVCROFT_NIX_E2E"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "present",
        "the flake dev shell's env var must be visible inside the session"
    );
}

#[test]
fn policy_render_shows_the_nix_store_grant_after_up() {
    let Some(sandbox) = Sandbox::new("policygrant", true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let out = sandbox.run(&["policy", "--render"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("/nix/store") && stdout.contains("provider:nix"),
        "got: {stdout}"
    );
}

#[test]
fn up_fails_at_provider_layer_when_flake_nix_is_missing() {
    if !nix_available() {
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
    }
    let project_root =
        std::env::temp_dir().join(format!("devcroft-nix-e2e-noflake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    let name = format!("e2enixnoflake{}", std::process::id());
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"nix\"\n"),
    )
    .unwrap();

    let out = run(&project_root, &["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nix flake init"), "got: {stderr}");

    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn up_fails_at_provider_layer_when_flake_lock_is_missing() {
    let Some(sandbox) = Sandbox::new("nolock", false) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nix flake lock"), "got: {stderr}");
}

#[test]
fn status_reports_stale_after_flake_nix_changes_and_up_suggests_recreate() {
    let Some(sandbox) = Sandbox::new("stale", true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());
    assert!(
        String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("env: fresh"),
        "must be fresh immediately after `up`"
    );

    // Touch flake.nix (content change, not just mtime — the fingerprint
    // is content-based) without re-locking, so this only exercises
    // staleness detection, not a real re-resolution.
    let mut flake = std::fs::read_to_string(sandbox.project_root.join("flake.nix")).unwrap();
    flake.push_str("\n# touched for staleness test\n");
    std::fs::write(sandbox.project_root.join("flake.nix"), flake).unwrap();

    let out = sandbox.run(&["status"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("env: stale") && stdout.contains("flake"),
        "got: {stdout}"
    );

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("already up"),
        "a plain `up` (no --recreate) on a stale-but-healthy keeper stays idempotent; \
         only `status` is expected to flag staleness — got {out:?}"
    );
}

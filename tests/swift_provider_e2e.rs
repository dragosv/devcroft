//! End-to-end coverage for the `swift` provider (`add-swift-provider`).
//!
//! **This provider is deliberately different from the other three**, and
//! the test's job is to hold it to what it actually claims rather than to
//! what a closure provider would claim:
//!
//! - it is **artifact** tier, so `up` and `status` must say so, and say
//!   what the tier costs rather than printing a bare word;
//! - resolving it **executes the project's own code**, so the disclosure
//!   warning must fire — and must *not* fire for a closure provider,
//!   which is the control that makes the first assertion a decision
//!   rather than the only thing devcroft knows how to print;
//! - every host path it depends on must appear in `policy --render` with
//!   a `provider:swift` origin, because "the artifact tier declares its
//!   host grants" is only true if you can see them.
//!
//! What this file does **not** assert is that `swift build` succeeds
//! inside the sandbox on macOS. It does not, and the reason is a
//! pre-existing published defect rather than anything this provider does
//! — see `docs/known-gaps.md`. Asserting a success that does not happen,
//! or quietly skipping the whole file to avoid saying so, are both worse
//! than stating it.

use std::process::Command;

fn swift_available() -> bool {
    Command::new("swift")
        .arg("-print-target-info")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A fixture package, dependency-free so it needs no `Package.resolved`
/// and no network — which is also the case that exercises the
/// conditional half of the lockfile rule.
fn write_fixture(root: &std::path::Path, name: &str) {
    std::fs::create_dir_all(root.join("Sources/app")).unwrap();
    std::fs::write(
        root.join("Package.swift"),
        "// swift-tools-version:5.9\nimport PackageDescription\n\
         let package = Package(name: \"app\", \
         targets: [.executableTarget(name: \"app\", path: \"Sources/app\")])\n",
    )
    .unwrap();
    // **Unguarded `import AppKit`, and it is load-bearing.** The provider
    // refuses a package a closure-tier provider could serve, so a fixture
    // importing only Foundation would be refused — correctly — and this
    // test would be asserting the wrong thing. The gate's own unit tests
    // cover the refusal; this fixture is the accepted side of it.
    std::fs::write(
        root.join("Sources/app/main.swift"),
        "import AppKit\nprint(\"hi\")\n",
    )
    .unwrap();
    std::fs::write(
        root.join("devcroft.toml"),
        format!("[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"swift\"\n"),
    )
    .unwrap();
}

#[test]
fn swift_resolves_as_artifact_tier_and_discloses_that_it_ran_project_code() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !swift_available() {
        eprintln!("skipping: no usable Swift toolchain on this host");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");
    let name = format!("swifte2e{}", std::process::id());
    let root = std::env::temp_dir().join(format!("devcroft-swift-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, &name);

    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .output()
        .unwrap();
    let up_stdout = String::from_utf8_lossy(&up.stdout).into_owned();
    let up_stderr = String::from_utf8_lossy(&up.stderr).into_owned();

    // **Not a skip.** Every precondition was checked above, so a failed
    // `up` here is a real failure — the rule `sandbox_environment.rs`
    // learned the hard way.
    assert!(
        up.status.success(),
        "`up` failed with a working Swift toolchain present:\nstdout: {up_stdout}\nstderr: {up_stderr}"
    );

    let status = Command::new(bin)
        .arg("status")
        .current_dir(&root)
        .output()
        .unwrap();
    let status_out = String::from_utf8_lossy(&status.stdout).into_owned();

    let render = Command::new(bin)
        .args(["policy", "--render"])
        .current_dir(&root)
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&render.stdout).into_owned();

    let env_out = Command::new(bin)
        .args(["exec", "--", "env"])
        .current_dir(&root)
        .output()
        .unwrap();
    let env_dump = String::from_utf8_lossy(&env_out.stdout).into_owned();

    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    let paths = devcroft::lifecycle::StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&root);

    // The tier, at `up` and in `status`, naming its cost both times.
    for (label, haystack) in [("up", &up_stdout), ("status", &status_out)] {
        assert!(
            haystack.contains("artifact"),
            "{label} must name the guarantee tier; got:\n{haystack}"
        );
        assert!(
            haystack.contains("host"),
            "{label} must say the artifact tier's runtime is host-linked, not \
             print the bare word; got:\n{haystack}"
        );
    }

    // The disclosure. This is the security-relevant assertion in the file:
    // resolving this provider ran `Package.swift`, which is project code.
    assert!(
        up_stderr.contains("ran this project's activation hook"),
        "resolving a swift environment executes Package.swift, and `up` must \
         say so; stderr was:\n{up_stderr}"
    );
    assert!(
        up_stderr.contains("swift"),
        "the disclosure must name the provider responsible; got:\n{up_stderr}"
    );

    // The artifact tier's host grants are visible, with their origin.
    assert!(
        rendered.contains("provider:swift"),
        "every host path this provider depends on must render with its own \
         origin; policy was:\n{rendered}"
    );

    // The environment devcroft resolved host-side reached the sandbox, so
    // nothing inside has to perform a host lookup of its own.
    assert!(
        env_dump.contains("SDKROOT=") || !cfg!(target_os = "macos"),
        "on macOS the SDK must be injected rather than looked up inside; \
         env was:\n{env_dump}"
    );
}

/// **The control.** Without this, an implementation that printed the
/// disclosure for every provider would pass the test above while telling
/// users nothing.
///
/// Uses `nix` because it is the provider whose hook-freedom is
/// *structural* — `print-dev-env --json` returns the `shellHook` as inert
/// data — so a disclosure here would be a real regression rather than a
/// coincidence of the fixture.
#[test]
fn a_closure_provider_makes_no_such_disclosure() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !devcroft::provider::host_can_build_nix_closures()
        || Command::new("nix").arg("--version").output().is_err()
    {
        eprintln!("skipping: no usable nix here (not on PATH, or no reachable Nix store)");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");
    let name = format!("swiftctl{}", std::process::id());
    let root = std::env::temp_dir().join(format!("devcroft-swift-ctl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("flake.nix"),
        // The same fixture shape `nix_provider_e2e.rs` uses: an explicit
        // system list, because `builtins.currentSystem` is unavailable in
        // a flake's pure evaluation.
        r#"
{
  description = "devcroft swift-control fixture";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      shellFor = system: (import nixpkgs { inherit system; }).mkShell {
        packages = [ (import nixpkgs { inherit system; }).bash ];
      };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("devcroft.toml"),
        format!("[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"nix\"\n"),
    )
    .unwrap();

    let lock = Command::new("nix")
        .args(["flake", "lock"])
        .current_dir(&root)
        .output();
    if !lock.is_ok_and(|o| o.status.success()) {
        eprintln!("skipping: `nix flake lock` failed on this host");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .output()
        .unwrap();
    let up_stdout = String::from_utf8_lossy(&up.stdout).into_owned();
    let up_stderr = String::from_utf8_lossy(&up.stderr).into_owned();

    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    if let Ok(paths) = devcroft::lifecycle::StatePaths::new(&name) {
        let _ = std::fs::remove_dir_all(&paths.root);
    }
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        up.status.success(),
        "the control's `up` failed:\n{up_stdout}\n{up_stderr}"
    );
    assert!(
        !up_stderr.contains("ran this project's activation hook"),
        "a closure provider must make no execution disclosure — otherwise the \
         swift assertion means nothing; stderr was:\n{up_stderr}"
    );
    assert!(
        up_stdout.contains("closure"),
        "and it must report the closure tier; stdout was:\n{up_stdout}"
    );
}

/// **The refusal, end to end through the real CLI.** The unit tests cover
/// the decision; this covers that it reaches the user as a `provider`-layer
/// failure with the alternative named, rather than as a sandbox that comes
/// up with a weaker guarantee nobody asked for.
#[test]
fn a_portable_swift_package_is_refused_and_pointed_at_a_closure_provider() {
    if !swift_available() {
        eprintln!("skipping: no usable Swift toolchain on this host");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");
    let name = format!("swiftport{}", std::process::id());
    let root = std::env::temp_dir().join(format!("devcroft-swift-portable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, &name);
    // The one difference from the accepted fixture: nothing Apple-only.
    std::fs::write(
        root.join("Sources/app/main.swift"),
        "import Foundation\nprint(\"hi\")\n",
    )
    .unwrap();

    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&up.stderr).into_owned();

    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    if let Ok(paths) = devcroft::lifecycle::StatePaths::new(&name) {
        let _ = std::fs::remove_dir_all(&paths.root);
    }
    let _ = std::fs::remove_dir_all(&root);

    assert!(!up.status.success(), "a portable package must not come up");
    assert_eq!(
        up.status.code(),
        Some(3),
        "a provider-layer refusal is exit 3 (error contract); stderr was:\n{stderr}"
    );
    assert!(
        stderr.contains("provider:"),
        "the failure must name its layer; got:\n{stderr}"
    );
    assert!(
        stderr.contains("nix") && stderr.contains("flox"),
        "a refusal must name the providers that serve this project better; got:\n{stderr}"
    );
}

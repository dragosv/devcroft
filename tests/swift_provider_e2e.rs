//! End-to-end coverage for the `swift` provider (`add-swift-provider`).
//!
//! **This provider is deliberately different from the other three**, and
//! the test's job is to hold it to what it actually claims rather than to
//! what a closure provider would claim:
//!
//! - it is **artifact** tier, so `up` and `status` must say so, and say
//!   what the tier costs rather than printing a bare word;
//! - resolving it opens **no project file at all** — the toolchain comes
//!   from `xcode-select`/`xcrun` — so the execution disclosure must stay
//!   silent, which is the opposite of what an earlier version of this
//!   provider did and the reason that version was wrong;
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

/// A usable toolchain **on macOS**. The platform is part of the
/// capability, not a separate check: GitHub's `ubuntu-latest` image ships
/// a Swift toolchain, so a probe on the binary alone said "available"
/// there and every test in this file then asserted a success the provider
/// refuses by design (it resolves Xcode or Command Line Tools and nothing
/// else). Measured on CI run 34869696777, all three failing. The Linux
/// case has its own test below, which uses that same toolchain to assert
/// the refusal.
fn swift_available() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    swift_binary_present()
}

fn swift_binary_present() -> bool {
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
    // **Unguarded `import AppKit`, and it is load-bearing.** A fixture
    // importing only Foundation draws `up`'s advice that a closure-tier
    // provider would serve it (a warning, or a refusal under
    // `require_native_apple_evidence`), and a test of the provider's
    // resolution should not be reading through that. The advice has its
    // own test below; this fixture is the evidence-present side of it.
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
fn swift_resolves_as_artifact_tier_without_reading_the_package_graph() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !swift_available() {
        eprintln!(
            "skipping: no usable Swift toolchain on this host, or not macOS (the provider is macOS-only)"
        );
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

    // **Inverted, and this is the security-relevant assertion in the
    // file.** An earlier version of this provider ran
    // `swift package dump-package` — which compiles and executes
    // `Package.swift` — to decide whether a lockfile was required, and
    // disclosed that faithfully. The disclosure was honest and the
    // execution was avoidable: resolving the *toolchain* needs no project
    // file, and dependency resolution belongs inside the sandbox.
    //
    // So the property is now the absence. If this assertion ever starts
    // failing, the provider has begun reading the package graph again and
    // has silently given up the cleanest criterion-4 position devcroft
    // has.
    assert!(
        !up_stderr.contains("ran this project's activation hook"),
        "the swift provider must open no project file, so nothing should be \
         disclosed; stderr was:\n{up_stderr}"
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

/// The evidence scan is advice by default and a gate only on request.
/// Both halves asserted in one place — a portable package comes up with
/// one warning that names the closure-tier route; the same package with
/// `require_native_apple_evidence = true` is refused at layer `provider`
/// with exit 3 — because either behaviour alone would pass a test written
/// for the other's opposite.
#[test]
fn a_portable_swift_package_is_advised_by_default_and_refused_on_request() {
    if !swift_available() {
        eprintln!(
            "skipping: no usable Swift toolchain on this host, or not macOS (the provider is macOS-only)"
        );
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

    // Default: honoured, with advice.
    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&up.stderr).into_owned();
    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    assert!(
        up.status.success(),
        "an explicit `provider = \"swift\"` must be honoured; stderr was:\n{stderr}"
    );
    assert_eq!(
        stderr.matches("warning: swift:").count(),
        1,
        "exactly one advice line; stderr was:\n{stderr}"
    );
    assert!(
        stderr.contains("nix")
            && stderr.contains("flox")
            && stderr.contains("require_native_apple_evidence"),
        "the advice must name the closure-tier route and the opt-in; got:\n{stderr}"
    );

    // Opt-in: refused, at the provider layer, before anything starts.
    std::fs::write(
        root.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"swift\"\n\
             require_native_apple_evidence = true\n"
        ),
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

    assert!(
        !up.status.success(),
        "with the opt-in, a portable package must not come up"
    );
    assert_eq!(
        up.status.code(),
        Some(3),
        "a provider-layer refusal is exit 3 (error contract); stderr was:\n{stderr}"
    );
    assert!(
        stderr.contains("provider:") && stderr.contains("require_native_apple_evidence = true"),
        "the refusal must name its layer and the key that asked for it; got:\n{stderr}"
    );
}

/// The build itself, through the shim, with the project root as the
/// manifest's only grant. This is the measurement the review asked for
/// before calling the provider a development environment, and it was
/// false on the branch as merged: SwiftPM's own Seatbelt cannot nest
/// inside devcroft's, and the manifest compile wrote to the Darwin
/// per-user cache directory. Both are now handled by what the provider
/// injects — the shim's two flags and the cache variables — so the
/// assertion is on a plain `swift build` and `swift run`, exactly as a
/// user would type them.
///
/// Xcode or Command Line Tools, whichever `xcode-select` names: the two
/// differ in what the toolchain needs granted (Xcode adds the bundle,
/// the license record, `/Library/Apple`), and this test is what tells a
/// host with one of them that the grants for it are complete.
#[test]
fn swift_build_and_run_succeed_through_the_shim_with_only_the_project_granted() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !swift_available() {
        eprintln!(
            "skipping: no usable Swift toolchain on this host, or not macOS (the provider is macOS-only)"
        );
        return;
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");
    let name = format!("swiftbuild{}", std::process::id());
    let root = std::env::temp_dir().join(format!("devcroft-swift-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    write_fixture(&root, &name);
    std::fs::write(
        root.join("Sources/app/main.swift"),
        "import AppKit\nprint(\"built-inside:\\(NSApplication.shared.isActive ? 1 : 0)\")\n",
    )
    .unwrap();

    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        up.status.success(),
        "up must succeed; stderr:\n{}",
        String::from_utf8_lossy(&up.stderr)
    );
    let stdout = String::from_utf8_lossy(&up.stdout);
    assert!(
        stdout.contains("swift build|run|test|package") && stdout.contains("--disable-sandbox"),
        "up must say the shim exists and what it adds; stdout was:\n{stdout}"
    );

    // Resolved through PATH, not by path: that is what makes the shim the
    // `swift` a user's own command line reaches.
    let which = Command::new(bin)
        .args(["exec", "--", "sh", "-c", "command -v swift"])
        .current_dir(&root)
        .output()
        .unwrap();
    let which = String::from_utf8_lossy(&which.stdout).trim().to_string();
    assert!(
        which.ends_with("/.devcroft/swift/bin/swift"),
        "the shim must be the swift on PATH; got {which:?}"
    );

    let build = Command::new(bin)
        .args(["exec", "--", "swift", "build"])
        .current_dir(&root)
        .output()
        .unwrap();
    let run = Command::new(bin)
        .args(["exec", "--", "swift", "run"])
        .current_dir(&root)
        .output()
        .unwrap();

    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    if let Ok(paths) = devcroft::lifecycle::StatePaths::new(&name) {
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    assert!(
        build.status.success(),
        "a plain `swift build` must succeed inside the sandbox; stderr:\n{}\nstdout:\n{}",
        String::from_utf8_lossy(&build.stderr),
        String::from_utf8_lossy(&build.stdout)
    );
    let build_err = String::from_utf8_lossy(&build.stderr);
    assert!(
        !build_err.contains("couldn't create cache file") && !build_err.contains("sandbox_apply"),
        "no cache or nested-sandbox noise may remain; stderr:\n{build_err}"
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("built-inside:"),
        "`swift run` must execute the built program; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    // Everything the build wrote is inside the project — the products in
    // SwiftPM's conventional place, the caches in the provider's.
    assert!(
        root.join(".build/debug").exists(),
        "products land in .build/debug"
    );
    assert!(
        root.join(".devcroft/swift/cache/swiftpm").exists()
            && root.join(".devcroft/swift/cache/clang").exists(),
        "caches land under .devcroft/swift/cache"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Off macOS, an explicit `provider = "swift"` is refused at layer
/// `provider`, naming the platform and the closure-tier route — even on
/// a host that *has* a Swift toolchain, which GitHub's Linux runners do.
/// That is the case the requirement exists for: the same manifest must
/// not mean Xcode on one machine and a different toolchain on another.
#[test]
fn off_macos_swift_is_refused_with_the_platform_named() {
    if cfg!(target_os = "macos") {
        eprintln!("skipping: this is the macOS host; the refusal is for every other platform");
        return;
    }
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock support");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");
    let name = format!("swiftlinux{}", std::process::id());
    let root = std::env::temp_dir().join(format!("devcroft-swift-linux-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, &name);

    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&up.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&root);

    assert!(!up.status.success(), "swift must not come up off macOS");
    assert_eq!(
        up.status.code(),
        Some(3),
        "layer provider is exit 3; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("provider:") && stderr.contains("macOS"),
        "the refusal must name its layer and the platform; got:\n{stderr}"
    );
    assert!(
        stderr.contains("nix") && stderr.contains("flox"),
        "and the closure-tier route; got:\n{stderr}"
    );
    // The point of the test: the host's own swift was there to be
    // misused, and was not.
    if swift_binary_present() {
        assert!(
            !stderr.contains("not found"),
            "a present toolchain must not be reported as missing; got:\n{stderr}"
        );
    }
}

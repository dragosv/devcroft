//! A flox environment that has never been locked: resolved by flox at
//! setup, then brought up by devcroft.
//!
//! **This test needs a FloxHub login, and fails without one rather than
//! skipping.** It exists because the flox row stopped resolving: it copies
//! a checked-in, already-locked environment (`tests/fixtures/flox-row`),
//! since flox 1.4 will not resolve a manifest without a login and the row
//! had therefore never run on a host without one. That fixed the row and
//! moved one path out of it -- a project's *first* lock, which is what a
//! user who runs `flox init` and `flox install` and then `devcroft up`
//! goes through. This file keeps that path covered, and refuses to let it
//! pass quietly untested.
//!
//! **When it runs.** Only when the flox row is selected
//! (`DEVCROFT_TEST_PROVIDER=flox` or `all`); the default run is the nix row
//! and does not touch flox. Once selected:
//!
//! - flox absent, or no reachable nix store: fails under `=flox`, skips
//!   under `=all` -- the same rule `common::for_each_row` applies to rows.
//! - flox present but unable to resolve (no login): **fails, under both.**
//!   This is the difference from a row. The row's fixture makes a missing
//!   login harmless for every other test, so the one test that still needs
//!   it must not turn that into one more skip.

use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::path::Path;
use std::process::Command;

fn flox(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("flox")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn a_never_locked_flox_environment_resolves_and_comes_up() {
    let selection = std::env::var("DEVCROFT_TEST_PROVIDER").unwrap_or_default();
    if selection != "flox" && selection != "all" {
        eprintln!(
            "skipping: the flox row is not selected (DEVCROFT_TEST_PROVIDER=flox|all); \
             this test needs a FloxHub login"
        );
        return;
    }
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let flox_usable = Command::new("flox")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
        && devcroft::provider::host_can_build_nix_closures();
    if !flox_usable {
        assert!(
            selection == "all",
            "DEVCROFT_TEST_PROVIDER=flox, but flox is not on PATH or this host has no \
             reachable nix store"
        );
        eprintln!("skipping: flox is not on PATH, or this host has no reachable nix store");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let root = std::path::PathBuf::from(format!("/tmp/dcfx-floxfresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    struct Gone(std::path::PathBuf, String);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = devcroft::lifecycle::rm(&self.1);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let sandbox = format!("fxfloxfresh{}", std::process::id());
    let _gone = Gone(root.clone(), sandbox.clone());

    let init = flox(&root, &["init"]);
    assert!(init.status.success(), "flox init failed: {init:?}");
    // The resolution this file exists for. Its failure is the test's
    // failure, with the remedy, never a skip.
    let install = flox(&root, &["install", "bash", "coreutils"]);
    assert!(
        install.status.success(),
        "flox could not resolve a fresh manifest: {}\n\
         This test needs a FloxHub login (`flox auth login`; in CI, see \
         https://go.flox.dev/auth). It fails rather than skips on purpose: \
         every other flox test runs from a pre-locked fixture, so this is \
         the only place a project's first lock is exercised.",
        String::from_utf8_lossy(&install.stderr).trim()
    );

    let text = format!("[sandbox]\nname = {sandbox:?}\n\n[env]\nprovider = \"flox\"\n");
    std::fs::write(root.join("devcroft.toml"), &text).unwrap();
    let (manifest, _) = devcroft::config::parse(&text).unwrap();
    let paths = StatePaths::new(&sandbox).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);

    let outcome =
        up(&manifest, &root, &UpOptions::default()).unwrap_or_else(|e| panic!("up failed: {e}"));
    assert_eq!(outcome, UpOutcome::Started);

    // The environment is the one flox just resolved: `pwd` is coreutils'
    // binary, run inside the boundary, and lands in the project root.
    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(["exec", "--", "pwd"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        root.to_string_lossy()
    );

    down(&sandbox).unwrap();
}

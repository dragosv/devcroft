//! `fix-provisioning-hooks`: provider resolution runs host-side, before
//! any restriction exists, with the invoking user's full network and
//! filesystem access. It must therefore not execute project-supplied
//! shell — the two-phase rule's own stated justification is that this
//! phase runs "pinned tooling from a lockfile, not project code".
//!
//! Both shipped providers violated that, and neither test suite noticed,
//! because nothing ever asked. These are the tests that ask. Each writes
//! a sentinel file from an activation hook and asserts, after resolution,
//! that the file does not exist:
//!
//! - **nix**: a devShell `shellHook`. Fixable — `nix print-dev-env
//!   --json` hands the hook back as inert data, so devcroft never
//!   evaluates it.
//! - **flox**: `[hook].on-activate`. *Not* fixable — measured against
//!   flox 1.14.0, no `flox activate` mode suppresses it (`--mode run`,
//!   `--mode dev`, `--no-start-services` all run it). So the flox test
//!   asserts the honest fallback instead: the hook does run, and
//!   resolution reports that it did, so `up` can warn rather than
//!   staying silent about project code having executed unconfined.
//!
//! Self-skipping on missing tooling, gated on **only** what each test
//! needs — the nix test does not require flox and vice versa.

use devcroft::provider::{Provider, ProviderError};
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "devcroft-provisioning-hooks-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn nix_usable() -> bool {
    Command::new("nix")
        .arg("eval")
        .arg("--expr")
        .arg("1")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn flox_usable() -> bool {
    Command::new("flox").arg("--version").output().is_ok()
}

/// The sentinel an activation hook writes if it runs. Absolute, because
/// the hook's own working directory is not something this test controls.
fn sentinel(dir: &Path) -> PathBuf {
    dir.join("HOOK_RAN")
}

#[test]
fn nix_resolution_does_not_run_the_dev_shells_shell_hook() {
    if !nix_usable() {
        eprintln!("skipping: a flakes-enabled nix is not on PATH");
        return;
    }
    let dir = scratch("nix");
    let marker = sentinel(&dir);

    // A dev shell that provides nothing but a hook with an observable
    // side effect. `mkShell` with no packages still produces a full
    // stdenv environment, which is all the capture needs.
    std::fs::write(
        dir.join("flake.nix"),
        format!(
            r#"{{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  outputs = {{ self, nixpkgs }}:
    let
      system = builtins.currentSystem or "{arch}";
      pkgs = import nixpkgs {{ inherit system; }};
    in {{
      devShells.${{system}}.default = pkgs.mkShell {{
        packages = [ ];
        shellHook = ''
          echo ran > "{marker}"
        '';
      }};
    }};
}}
"#,
            arch = std::env::consts::ARCH.to_string() + "-linux",
            marker = marker.display(),
        ),
    )
    .unwrap();

    let lock = Command::new("nix")
        .arg("flake")
        .arg("lock")
        .current_dir(&dir)
        .output()
        .unwrap();
    if !lock.status.success() {
        eprintln!(
            "skipping: `nix flake lock` failed: {}",
            String::from_utf8_lossy(&lock.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let resolved = devcroft::provider::NixProvider.resolve(&dir);

    // Resolution itself must succeed — a hook that does not run must not
    // break the capture.
    let resolution = match resolved {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            panic!("nix resolution failed: {e}");
        }
    };
    assert!(
        !resolution.env.is_empty(),
        "the capture must still produce an environment"
    );

    let ran = marker.exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !ran,
        "the dev shell's shellHook ran during provider resolution — project code \
         executed host-side, before any restriction, with full network and filesystem \
         access (two-phase rule)"
    );
}

#[test]
fn flox_resolution_reports_that_an_activation_hook_ran() {
    if !flox_usable() {
        eprintln!("skipping: flox not on PATH");
        return;
    }
    let dir = scratch("flox");
    let marker = sentinel(&dir);

    let init = Command::new("flox")
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let manifest_path = dir.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest = manifest.replacen(
        "[hook]\n",
        &format!(
            "[hook]\non-activate = '''\n  echo ran > \"{}\"\n'''\n",
            marker.display()
        ),
        1,
    );
    std::fs::write(&manifest_path, manifest).unwrap();

    let resolution = match devcroft::provider::FloxProvider.resolve(&dir) {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            panic!("flox resolution failed: {e}");
        }
    };

    // flox offers no way to capture without running the hook, so the
    // requirement is not "it did not run" but "devcroft knows it did"
    // — which is what lets `up` warn instead of staying silent.
    let ran = marker.exists();
    let reported = resolution.ran_activation_hook;
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        ran,
        "sanity: flox is expected to run on-activate during capture; if this now \
         fails, flox gained a way to avoid it and the provider should use it"
    );
    assert!(
        reported,
        "resolution ran the project's on-activate hook host-side but did not report \
         it, so `up` cannot warn — the silent case this change exists to remove"
    );
}

/// The converse, and the one that keeps the warning meaningful: an
/// environment with no hook must not be reported as having run one.
#[test]
fn flox_resolution_reports_nothing_when_there_is_no_hook() {
    if !flox_usable() {
        eprintln!("skipping: flox not on PATH");
        return;
    }
    let dir = scratch("floxnohook");

    let init = Command::new("flox")
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!("skipping: flox init failed");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    // `flox init`'s stock manifest has a `[hook]` section with
    // `on-activate` present only as a comment — the distinction this
    // test pins down, since a naive "does the file contain
    // on-activate" check would report a hook that does not exist.
    let resolution = match devcroft::provider::FloxProvider.resolve(&dir) {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            panic!("flox resolution failed: {e}");
        }
    };
    let reported = resolution.ran_activation_hook;
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !reported,
        "a manifest whose on-activate is only a comment must not be reported as \
         running a hook — a warning that always fires is one users stop reading"
    );
}

/// Keeps the unused-import lint quiet while documenting that these tests
/// deliberately exercise the real `Provider` trait rather than a double.
#[allow(dead_code)]
fn _assert_error_type(_: ProviderError) {}

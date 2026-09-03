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
//! - **flox**: `[hook].on-activate`. Not suppressible *by flox* — measured
//!   against flox 1.14.0, no activation mode skips it (`--mode run`,
//!   `--mode dev`, `--no-start-services` all run it). It is nonetheless
//!   fixed, by devcroft rather than upstream: materialization runs against
//!   a derived hook-free copy of the environment, and the hook runs inside
//!   the sandbox afterwards (`sandbox-provisioning` P2d, exercised in
//!   `tests/flox_derived_env.rs`).
//!
//! **This file's title is now true of every provider**, which it was not
//! when written. The flox test below used to assert the honest fallback —
//! the hook runs, and resolution *reports* that it did, so `up` can warn.
//! Its own sanity assertion carried the trigger for revisiting it: "if this
//! now fails, flox gained a way to avoid it and the provider should use
//! it". It did fail, for a near-miss reason worth recording — devcroft
//! constructed the way rather than flox providing one — and the assertions
//! are inverted accordingly.
//!
//! Self-skipping on missing tooling, gated on **only** what each test
//! needs — the nix test does not require flox and vice versa.

use devcroft::provider::{Provider, ProviderError};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Nix's own system double for this host.
///
/// Not a hardcoded `-linux`: on an Apple Silicon host nix asks for
/// `aarch64-darwin`, and a flake declaring only `aarch64-linux` fails
/// evaluation with "does not provide attribute
/// devShells.aarch64-darwin.default" — a fixture bug that reads exactly
/// like a provider regression. `builtins.currentSystem` is unavailable in
/// pure flake evaluation, so this literal is what actually gets used.
fn nix_system_double() -> String {
    format!(
        "{}-{}",
        std::env::consts::ARCH,
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        }
    )
}

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
        && devcroft::provider::host_can_build_nix_closures()
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
            arch = nix_system_double(),
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
fn flox_resolution_does_not_run_the_projects_activation_hook() {
    if !flox_usable() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
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

    // The requirement is now the same one the other providers meet: the
    // project's hook must not have executed on the host at all.
    let ran = marker.exists();
    let reported = resolution.ran_activation_hook;
    let captured = resolution.activation_script.is_some();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !ran,
        "the project's on-activate hook executed on the host during resolution — \
         the derived hook-free environment (P2d) is what must prevent this, and \
         a failure here means resolution fell back to the project's own \
         environment"
    );
    assert!(
        !reported,
        "nothing project-supplied ran unconfined, so resolution must not report \
         that it did — this field means 'did project code execute on the host', \
         and a stale `true` would make `up` warn about something that did not \
         happen"
    );
    assert!(
        captured,
        "the hook must still be captured as data, or it would silently never run \
         at all — which would break every project whose hook does real setup"
    );
}

/// The converse, and the one that keeps the warning meaningful: an
/// environment with no hook must not be reported as having run one.
#[test]
fn flox_resolution_reports_no_hook_when_there_is_none() {
    if !flox_usable() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
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

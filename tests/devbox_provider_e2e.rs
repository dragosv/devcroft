//! End-to-end coverage for the `devbox` provider — add-devbox-provider's
//! task group 3 — through the real built binary against a real `devbox`
//! plus `nix` sandbox, same pattern `nix_provider_e2e.rs` uses. Skips
//! quietly, like every other real-tooling test in this suite, wherever
//! `devbox` (or the `nix` it depends on) isn't on `PATH`.

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

fn devbox_available() -> bool {
    Command::new("devbox")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
        && Command::new("nix").arg("--version").output().is_ok()
}

struct Sandbox {
    name: String,
    project_root: PathBuf,
}

impl Sandbox {
    /// `packages`: names to declare in `devbox.json` (may be empty).
    /// `locked`: whether to run `devbox install`, producing a complete
    /// lockfile — `false` leaves the project unlocked, for the
    /// missing-lock failure tests.
    ///
    /// `install` runs even with **no** packages declared, which is not
    /// redundant: devbox's base nixpkgs entry is itself a resolution, so
    /// a zero-package project with no lockfile still resolves the
    /// floating `nixpkgs-unstable` branch at `up` — which `resolve` now
    /// refuses (see `provider::devbox::restore_lock_if_capture_resolved`).
    fn new(tag: &str, packages: &[&str], locked: bool) -> Option<Self> {
        if !devbox_available() {
            eprintln!("skipping: devbox or nix not on PATH");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }

        let project_root =
            std::env::temp_dir().join(format!("devcroft-devbox-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&project_root);
        std::fs::create_dir_all(&project_root).unwrap();

        let packages_json = packages
            .iter()
            .map(|p| format!("{p:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        std::fs::write(
            project_root.join("devbox.json"),
            format!(r#"{{"packages": [{packages_json}]}}"#),
        )
        .unwrap();

        if locked {
            let install = Command::new("devbox")
                .arg("install")
                .current_dir(&project_root)
                .output()
                .unwrap();
            if !install.status.success() {
                eprintln!(
                    "skipping: devbox install failed (likely no network for nixpkgs): {}",
                    String::from_utf8_lossy(&install.stderr)
                );
                return None;
            }
        }

        let name = format!("e2edevbox{tag}{}", std::process::id());
        std::fs::write(
            project_root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"devbox\"\n"),
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

/// Spec: "Toolchain from the devbox environment is visible in a session."
#[test]
fn up_resolves_devbox_and_the_toolchain_is_visible_in_a_session() {
    let Some(sandbox) = Sandbox::new("up", &["ripgrep@latest"], true) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("is started"));

    // network.default defaults to deny (config::Network's Default impl) —
    // materialization already happened host-side at `up` (spec: "Toolchain
    // works under network deny-all"), so no session-time network is
    // needed for this to work.
    let out = sandbox.run(&["exec", "--", "rg", "--version"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ripgrep"),
        "expected the devbox-resolved ripgrep to run inside the session"
    );
}

/// Spec: "Store paths become readable", with origin `provider:devbox`.
#[test]
fn policy_render_shows_the_devbox_store_grant_after_up() {
    let Some(sandbox) = Sandbox::new("policygrant", &["ripgrep@latest"], true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let out = sandbox.run(&["policy", "--render"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("/nix/store") && stdout.contains("provider:devbox"),
        "got: {stdout}"
    );
}

/// Spec: "Missing environment, not missing feature."
#[test]
fn up_fails_at_provider_layer_when_devbox_json_is_missing() {
    if !devbox_available() {
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
    }
    let project_root = std::env::temp_dir().join(format!(
        "devcroft-devbox-e2e-nodevboxjson-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    let name = format!("e2edevboxnoenv{}", std::process::id());
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"devbox\"\n"),
    )
    .unwrap();

    let out = run(&project_root, &["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("devbox init"), "got: {stderr}");

    let _ = std::fs::remove_dir_all(&project_root);
}

/// Spec: "Declared but unlocked package fails rather than resolving."
#[test]
fn up_fails_at_provider_layer_when_a_declared_package_has_no_lockfile() {
    let Some(sandbox) = Sandbox::new("nolock", &["ripgrep@latest"], false) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("devbox install"), "got: {stderr}");
}

/// Spec: "A project declaring no packages still needs a lockfile."
///
/// Corrected by adversarial review — an earlier version of this test
/// asserted the opposite ("needs no lockfile"), matching a spec scenario
/// since replaced. devbox's stdenv comes from its base nixpkgs, which is
/// unpinned without a lockfile, so a zero-package project is reproducible
/// only once `devbox install` has written one.
#[test]
fn up_succeeds_with_no_declared_packages_once_the_base_is_locked() {
    let Some(sandbox) = Sandbox::new("zeropkg", &[], true) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
}

/// The regression adversarial review found in the shipped code: `up`
/// rewrote the user's `devbox.lock` during provisioning — the exact thing
/// the `env-provider` spec says resolution SHALL NOT do. Here the project
/// declares nothing and has no lockfile at all, so capture would create
/// one; `up` must refuse and leave the tree as it found it.
#[test]
fn up_refuses_rather_than_writing_a_lockfile_during_provisioning() {
    let Some(sandbox) = Sandbox::new("nolockwrite", &[], false) else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("devbox install"), "got: {stderr}");
    assert!(
        !sandbox.project_root.join("devbox.lock").is_file(),
        "a refused `up` must not leave behind the lockfile capture created"
    );
}

/// Spec: "Stale environment after devbox file change."
#[test]
fn status_reports_stale_after_devbox_json_changes_and_up_suggests_recreate() {
    let Some(sandbox) = Sandbox::new("stale", &["ripgrep@latest"], true) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());
    assert!(
        String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("env: fresh"),
        "must be fresh immediately after `up`"
    );

    // Touch devbox.json (content change) without re-locking, so this only
    // exercises staleness detection, not a real re-resolution.
    let mut manifest = std::fs::read_to_string(sandbox.project_root.join("devbox.json")).unwrap();
    manifest = manifest
        .replace("packages", "env")
        .replace("env", "packages");
    manifest.push('\n');
    std::fs::write(sandbox.project_root.join("devbox.json"), manifest).unwrap();

    let out = sandbox.run(&["status"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("env: stale") && stdout.contains("devbox"),
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

/// The measurement that earns the "closure tier" claim for devbox
/// specifically (env-provider spec: "Toolchain works under network
/// deny-all"; `own-policy-baseline` recorded the same measurement for
/// flox and nix). Without this, "closure tier" would be an inference from
/// devbox being Nix-backed rather than a fact checked about this
/// provider's own resolved environment.
#[test]
fn a_real_build_succeeds_from_the_devbox_closure_with_the_host_toolchain_denied() {
    if !std::path::Path::new("/usr/bin/gcc").is_file() {
        eprintln!("skipping: no host /usr/bin/gcc to assert denial against");
        return;
    }
    let Some(sandbox) = Sandbox::new("gccbuild", &["gcc@latest"], true) else {
        return;
    };
    // `/tmp` is not part of what devcroft grants a closure-tier project by
    // default (own-policy-baseline; see `samples/nix-go-sample`'s own
    // manifest for the identical need) — gcc-wrapper's intermediate
    // compilation files live there, so the project must declare it like
    // any other filesystem need.
    let mut manifest = std::fs::read_to_string(sandbox.project_root.join("devcroft.toml")).unwrap();
    manifest.push_str("\n[filesystem]\nallow = [\".\", \"/tmp\"]\n");
    std::fs::write(sandbox.project_root.join("devcroft.toml"), manifest).unwrap();

    assert!(sandbox.run(&["up"]).status.success());

    // The host's own gcc must be unreachable — the baseline grants no
    // host toolchain paths (own-policy-baseline). Actually invoking it,
    // not `command -v`: a bare path-existence check can succeed under
    // Landlock even when execution is denied, since stat and exec are
    // mediated separately — the claim is about exec, so the test must
    // exercise exec.
    let out = sandbox.run(&["exec", "--", "/usr/bin/gcc", "--version"]);
    assert!(
        !out.status.success(),
        "expected /usr/bin/gcc to be denied inside the sandbox, but it ran: {out:?}"
    );

    // The devbox-resolved gcc must compile and run a real program, with
    // network.default staying deny (materialization already happened
    // host-side at `up`).
    let write_source = sandbox.run(&[
        "exec",
        "--",
        "sh",
        "-c",
        "printf '#include <stdio.h>\\nint main(){printf(\"devbox-build-ok\\\\n\");return 0;}' > hello.c",
    ]);
    assert!(write_source.status.success(), "{write_source:?}");

    let compile_and_run =
        sandbox.run(&["exec", "--", "sh", "-c", "gcc -o hello hello.c && ./hello"]);
    assert!(compile_and_run.status.success(), "{compile_and_run:?}");
    assert!(
        String::from_utf8_lossy(&compile_and_run.stdout).contains("devbox-build-ok"),
        "got: {compile_and_run:?}"
    );
}

//! The obligations `add-devenv-provider`'s design took on in exchange for
//! consuming an artifact devenv documents as internal (decision 2) and
//! for filtering that artifact's contents (decision 8).
//!
//! Both are promises about a format devcroft does not control, so both
//! are pinned here against a **real** devenv rather than against a
//! recorded sample: a sample would keep passing after the upstream change
//! it exists to catch. Skips on the capability, never on the binary.

use devcroft::provider::{DevenvProvider, Provider};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn devenv_can_resolve() -> bool {
    Command::new("devenv")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
        && Command::new("nix").arg("--version").output().is_ok()
        && devcroft::provider::host_can_build_nix_closures()
}

/// A locked devenv project, or `None` where the inputs cannot be fetched.
fn locked_project(tag: &str) -> Option<PathBuf> {
    if !devenv_can_resolve() {
        eprintln!("skipping: devenv or nix not usable on this host");
        return None;
    }
    let root = std::env::temp_dir().join(format!(
        "devcroft-devenv-contract-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("devenv.nix"),
        "{ pkgs, ... }: {\n  packages = [ pkgs.ripgrep ];\n  env.DEVCROFT_CONTRACT = \"pinned\";\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("devenv.yaml"),
        "inputs:\n  nixpkgs:\n    url: github:cachix/devenv-nixpkgs/rolling\n",
    )
    .unwrap();
    let updated = Command::new("devenv")
        .arg("update")
        .current_dir(&root)
        .output()
        .unwrap();
    if !updated.status.success() {
        eprintln!(
            "skipping: devenv update failed (likely no network for nixpkgs): {}",
            String::from_utf8_lossy(&updated.stderr)
        );
        let _ = std::fs::remove_dir_all(&root);
        return None;
    }
    Some(root)
}

/// `devenv`'s absolute path, from the ambient `PATH`.
fn which_devenv() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("devenv"))
            .find(|candidate| candidate.is_file())
    })
}

/// The environment as a developer actually gets it: through the route
/// that **does** run `enterShell`. Only a test may take this route —
/// `resolve` never does — and it exists so the filtered hook-free capture
/// has something real to be checked against.
fn truth_environment(root: &Path) -> BTreeMap<String, String> {
    // Absolute, because the canonical baseline below does not contain
    // whatever directory `devenv` lives in — `env_clear` plus a fixed
    // `PATH` is exactly what the provider's own capture does, and
    // resolving the program by name against it would fail.
    let devenv = which_devenv().expect("devenv is on PATH for this test to have run at all");
    let out = Command::new(&devenv)
        .args(["shell", "--", "env", "-0"])
        .current_dir(root)
        .env_clear()
        .env("HOME", std::env::var("HOME").unwrap())
        .env(
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        )
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter_map(|e| e.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// design.md decision 2, second obligation: an upstream change to
/// `devenv build shell`'s output breaks CI rather than a user's sandbox.
///
/// Asserts the two properties the parser depends on — the artifact is
/// reachable through the documented JSON key, and its body is `declare
/// -x` lines under a banner — plus that the project's own declarations
/// survive it. A parser failure here is the *intended* outcome of an
/// upstream format change; a silently smaller environment is not.
#[test]
fn the_capture_artifact_still_has_the_shape_devcroft_parses() {
    let Some(root) = locked_project("format") else {
        return;
    };

    let resolution = DevenvProvider.resolve(&root).unwrap();

    assert_eq!(
        resolution.env.get("DEVCROFT_CONTRACT").map(String::as_str),
        Some("pinned"),
        "the project's own `env` must survive capture"
    );
    assert!(
        resolution
            .env
            .get("PATH")
            .is_some_and(|p| p.contains("/nix/store/")),
        "the captured PATH must come from the closure"
    );
    assert_eq!(
        resolution.read_only_grants,
        vec!["/nix/store".to_string()],
        "closure tier: the store root is the grant, and nothing else is"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// design.md decision 8's pin: the filtered hook-free capture must not
/// carry anything the real environment does not, apart from an enumerated
/// set this test names.
///
/// The enumeration is the point. A key on neither side of it fails here,
/// which is the only mechanism that keeps a filter written against one
/// devenv version honest against the next.
#[test]
fn the_filtered_capture_carries_nothing_the_real_environment_lacks() {
    let Some(root) = locked_project("filter") else {
        return;
    };

    let resolution = DevenvProvider.resolve(&root).unwrap();
    let truth = truth_environment(&root);

    // Legitimately present in the capture and absent from the truth,
    // each for a stated reason rather than by tolerance.
    let expected_extra: &[&str] = &[
        // Toolchain configuration a build needs; the hook-running route
        // keeps it too, under a different random seed per derivation.
        "NIX_CFLAGS_COMPILE",
        "NIX_CFLAGS_LINK",
        "NIX_LDFLAGS",
        // devenv's own state, read by the captured hook.
        "DEVENV_CMDLINE",
        // Points at a store bash rather than at the host's login shell,
        // which is the better answer for a sandbox and the one
        // `src/shell.rs` wants.
        "SHELL",
    ];

    let unexpected: Vec<&String> = resolution
        .env
        .keys()
        .filter(|k| !truth.contains_key(*k))
        .filter(|k| !expected_extra.contains(&k.as_str()))
        .filter(|k| !k.starts_with("NIX_CFLAGS") && !k.starts_with("NIX_LDFLAGS"))
        .collect();

    assert!(
        unexpected.is_empty(),
        "the capture carries variables the real devenv environment does not, and they are \
         not in this test's enumerated set: {unexpected:?}. Either the filter in \
         `provider::devenv::BUILDER_VARIABLES` needs the new name, or the enumeration \
         above does — decide which, do not widen the filter by reflex."
    );

    // The three that are not untidiness but breakage, named individually
    // so a regression says which one came back.
    assert_ne!(
        resolution.env.get("HOME").map(String::as_str),
        Some("/homeless-shelter"),
        "the builder's sentinel home must never reach a session"
    );
    assert!(
        !resolution.env.contains_key("SSL_CERT_FILE"),
        "the builder's /no-cert-file.crt would break TLS in the sandbox"
    );
    assert!(
        !resolution
            .env
            .get("TMPDIR")
            .is_some_and(|t| t.contains("/nix/var/nix/builds")),
        "a temp directory inside a Nix build tree does not exist at session time"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// The other half of the same pin, and it must be checked separately: a
/// filter that dropped everything would pass the test above.
#[test]
fn the_filtered_capture_still_carries_what_the_real_environment_has() {
    let Some(root) = locked_project("nothole") else {
        return;
    };

    let resolution = DevenvProvider.resolve(&root).unwrap();
    let truth = truth_environment(&root);

    // Set by `enterShell`, which devcroft runs *inside* the sandbox
    // rather than during capture — so their absence from the capture is
    // the design working, not a hole.
    let restored_by_the_hook: &[&str] = &["IN_NIX_SHELL", "MANPATH", "DEVENV_CMDLINE"];
    // Shell bookkeeping that belongs to whatever process is running, not
    // to the environment being captured.
    let shell_bookkeeping: &[&str] = &["OLDPWD", "PWD", "SHLVL", "_", "__CF_USER_TEXT_ENCODING"];
    // Present in the real environment and dropped on purpose. Both are
    // leakage from the tooling that produced the shell rather than
    // anything the project declared, and neither is reproducible:
    //
    // - `name` is the derivation's own attribute, and the two routes
    //   disagree about its value (`devenv-shell` against
    //   `devenv-shell-env`) — so carrying it would make the captured
    //   environment depend on which route produced it.
    // - `GC_LARGE_ALLOC_WARN_INTERVAL` is a Boehm GC knob Nix sets for
    //   its *own* evaluator, which then leaks into the shell it built.
    //
    // Listed rather than silently tolerated: if either ever becomes
    // something a project can set, this entry is where that gets noticed.
    let deliberately_dropped: &[&str] = &["name", "GC_LARGE_ALLOC_WARN_INTERVAL"];

    let missing: Vec<&String> = truth
        .keys()
        .filter(|k| !resolution.env.contains_key(*k))
        .filter(|k| !restored_by_the_hook.contains(&k.as_str()))
        .filter(|k| !shell_bookkeeping.contains(&k.as_str()))
        .filter(|k| !deliberately_dropped.contains(&k.as_str()))
        // The baseline's own keys are not "changed by activation" and so
        // are legitimately absent from the diff.
        .filter(|k| *k != "HOME" && *k != "PATH")
        .collect();

    assert!(
        missing.is_empty(),
        "the capture is missing variables the real devenv environment has: {missing:?}. \
         If the hook sets them, add them to `restored_by_the_hook` with that reason; \
         otherwise the capture route has a hole and the provider must fail rather than \
         report a partial environment as success."
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Mirrors `devbox_env_capture_is_deterministic.rs`: the fixed baseline in
/// `provider::capture` is shared, not reimplemented per provider, and a
/// decoy on the invoking shell must not reach the captured diff.
///
/// Mutates this process's own `PATH`, so it lives in its own test binary
/// alongside tests that do not depend on `PATH` — same reasoning the flox,
/// nix and devbox versions give for standing alone.
#[test]
fn a_decoy_on_the_invoking_shell_does_not_leak_into_the_capture() {
    let Some(root) = locked_project("determinism") else {
        return;
    };

    let decoy = root.join("decoy-bin");
    std::fs::create_dir_all(&decoy).unwrap();

    let before = DevenvProvider.resolve(&root).unwrap();

    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{}", decoy.display(), std::env::var("PATH").unwrap()),
        );
        std::env::set_var("DEVCROFT_DECOY_MARKER", "leaked");
    }

    let after = DevenvProvider.resolve(&root).unwrap();

    assert_eq!(
        before.env, after.env,
        "the captured diff must not depend on the invoking shell's environment"
    );
    assert!(
        !after.env.contains_key("DEVCROFT_DECOY_MARKER"),
        "an env var on the invoking shell leaked into the capture"
    );
    assert!(
        !after
            .env
            .get("PATH")
            .is_some_and(|p| p.contains("decoy-bin")),
        "a PATH entry on the invoking shell leaked into the capture"
    );

    let _ = std::fs::remove_dir_all(&root);
}

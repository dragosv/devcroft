//! `sandbox-provisioning` P2d: a flox environment declaring
//! `[hook].on-activate` is materialized from a **derived, hook-free
//! copy**, so the project's activation code never runs unconfined on the
//! host — and then runs inside the sandbox instead.
//!
//! Before this, flox was the one provider whose project code executed
//! host-side during `up`, before any boundary existed. That made confined
//! provisioning meaningless for the provider devcroft recommends by
//! default and `init` scaffolds.
//!
//! The property everything rests on, and the one this file exists to
//! guard: **stripping `[hook]` does not change what gets materialized.**
//! A hook is not a package input, so removing it cannot alter the
//! resolved closure. If that ever stops holding, devcroft would be
//! silently materializing something the project did not declare — so the
//! closure identity is asserted directly rather than inferred from
//! activation merely succeeding.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use devcroft::provider::{Provider, ProviderKind};
use std::path::{Path, PathBuf};
use std::process::Command;

fn flox_available() -> bool {
    Command::new("flox")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A flox project whose hook leaves evidence on disk if it ever runs.
/// The marker is per-process so a stale file from an earlier failing run
/// cannot make a later run pass or fail spuriously.
fn flox_project(tag: &str, hook_body: Option<&str>) -> Option<(PathBuf, PathBuf)> {
    let root = std::env::temp_dir().join(format!(
        "devcroft-flox-derived-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    if !Command::new("flox")
        .arg("init")
        .current_dir(&root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        eprintln!("skipping: flox init failed");
        return None;
    }

    let marker = root.join("HOOK_RAN_ON_HOST");
    let mut manifest = String::from("version = 1\n\n[install]\njq.pkg-path = \"jq\"\n");
    if let Some(body) = hook_body {
        manifest.push_str(&format!(
            "\n[hook]\non-activate = '''\n{}\n'''\n",
            body.replace("{MARKER}", &marker.display().to_string())
        ));
    }
    std::fs::write(root.join(".flox/env/manifest.toml"), manifest).unwrap();
    Some((root, marker))
}

/// What a project's toolchain actually resolves to, via the provider.
fn resolved_jq(root: &Path) -> Option<String> {
    let resolution = ProviderKind::from_name("flox").ok()?.resolve(root).ok()?;
    let path = resolution.env.get("PATH")?;
    for entry in path.split(':') {
        let candidate = Path::new(entry).join("jq");
        if candidate.exists() {
            return std::fs::canonicalize(candidate)
                .ok()
                .map(|p| p.display().to_string());
        }
    }
    None
}

#[test]
fn a_hook_does_not_run_on_the_host_during_resolution() {
    if !flox_available() {
        eprintln!("skipping: flox not on PATH");
        return;
    }
    let Some((root, marker)) = flox_project("nohost", Some("touch {MARKER}")) else {
        return;
    };

    let resolution = ProviderKind::from_name("flox")
        .unwrap()
        .resolve(&root)
        .unwrap_or_else(|e| panic!("resolve failed: {e}"));

    assert!(
        !marker.exists(),
        "the project's [hook].on-activate must not execute on the host during resolution"
    );
    assert!(
        resolution.activation_script.is_some(),
        "the hook must be captured as data so it can run inside the sandbox instead"
    );
    assert!(
        !resolution.ran_activation_hook,
        "nothing project-supplied ran unconfined, so this must report false"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// The load-bearing property. Two projects with an identical `[install]`
/// section, one with a hook and one without, must resolve to the same
/// store path — proving the derived environment materializes what the
/// project declared and not something else.
#[test]
fn the_derived_environment_resolves_to_an_identical_closure() {
    if !flox_available() {
        eprintln!("skipping: flox not on PATH");
        return;
    }
    let Some((with_hook, _)) = flox_project("closure-hook", Some("touch {MARKER}")) else {
        return;
    };
    let Some((without_hook, _)) = flox_project("closure-plain", None) else {
        return;
    };

    let derived = resolved_jq(&with_hook);
    let direct = resolved_jq(&without_hook);

    assert!(
        derived.is_some(),
        "the hooked project must still resolve jq"
    );
    assert_eq!(
        derived, direct,
        "stripping [hook] must not change the resolved closure — if this fails, \
         devcroft is materializing something the project did not declare"
    );

    let _ = std::fs::remove_dir_all(&with_hook);
    let _ = std::fs::remove_dir_all(&without_hook);
}

#[test]
fn a_project_without_a_hook_takes_the_unchanged_path() {
    if !flox_available() {
        eprintln!("skipping: flox not on PATH");
        return;
    }
    let Some((root, _)) = flox_project("nohook", None) else {
        return;
    };

    let resolution = ProviderKind::from_name("flox")
        .unwrap()
        .resolve(&root)
        .unwrap_or_else(|e| panic!("resolve failed: {e}"));

    assert!(resolution.activation_script.is_none());
    // No derived copy should exist: deriving one for an environment with
    // nothing to strip would be pure cost, and would put a directory in
    // the project that nothing reads.
    assert!(
        !root.join(".devcroft").exists()
            || std::fs::read_dir(root.join(".devcroft"))
                .map(|d| d
                    .filter_map(Result::ok)
                    .all(|e| { !e.file_name().to_string_lossy().starts_with("flox-env-") }))
                .unwrap_or(true),
        "a project with no hook must not get a derived environment"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// End to end: the hook must still *happen*, just inside the boundary.
/// Materializing without it would otherwise silently break every project
/// whose hook does real setup — which is most of them.
#[test]
fn the_hook_runs_inside_the_sandbox_instead() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !flox_available() {
        eprintln!("skipping: flox not on PATH");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let Some((root, marker)) = flox_project("insandbox", Some("touch {MARKER}")) else {
        return;
    };
    // The hook's own commands must come from the closure, not the host.
    // Worth stating because getting this wrong is how the feature first
    // *looked* broken: with only `bash` installed, the hook failed with
    // `sh: /usr/bin/touch: Permission denied` — Landlock refusing a host
    // binary, which is `own-policy-baseline` working exactly as intended
    // and is the whole point of running the hook in here. A hook that
    // reaches for host tooling is now denied; before this change it would
    // have run on the host with full access.
    let _ = Command::new("flox")
        .args(["install", "bash", "coreutils"])
        .current_dir(&root)
        .output();

    let sandbox_name = format!("e2efloxhook{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let outcome = up(&manifest, &root, &UpOptions::default());
    match outcome {
        Ok(UpOutcome::Started) => {
            assert!(
                marker.exists(),
                "the hook must still run — inside the sandbox — or every project \
                 whose hook does setup silently breaks"
            );
        }
        Ok(other) => panic!("expected Started, got {other:?}"),
        Err(e) => panic!("up failed: {e}"),
    }

    let _ = down(&sandbox_name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&root);
}

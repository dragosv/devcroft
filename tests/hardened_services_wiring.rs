//! Tier parity for provider-declared services (add-flox-services task
//! 3.2: "do not add a tier-specific path"). The failure this guards
//! against is not a wrong result but an *absent* one: `up_hardened`
//! originally had no services block at all, so a project declaring
//! Postgres came up at `isolation = "hardened"` reporting a healthy
//! sandbox, a fresh environment, and zero service lines — indistinguishable
//! from a project that declares no services. Nothing failed; the seam
//! decision 2 exists to provide simply was never called on that branch.
//!
//! This asserts the host-side half of that wiring, which is the half a
//! machine without a functional `runsc` can still observe:
//! `prepare_services` runs on the hardened path, and it runs *before*
//! any sandbox is started. `tests/gvisor_hardened_e2e.rs` documents why
//! this repo's own devcontainer cannot run `runsc` at all (rootless
//! re-exec into a user namespace is `EPERM` here), so the in-sandbox
//! half — process-compose actually supervised via `runsc exec` — is
//! covered by the `services_e2e` shape only at the process tier for now,
//! and is called out as unverified in tasks.md rather than claimed.
//!
//! See `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, up};
use std::process::Command;

fn tooling_missing() -> bool {
    Command::new("flox").arg("--version").output().is_err()
        || devcroft::gvisor::runsc_command::resolve().is_none()
}

#[test]
fn hardened_up_enforces_the_process_compose_requirement_before_starting_a_sandbox() {
    if tooling_missing() {
        eprintln!("skipping: flox and/or runsc not on PATH");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root =
        std::env::temp_dir().join(format!("devcroft-hardened-svc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    let init = Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        return;
    }

    // A declared service, and deliberately *no* `flox install
    // process-compose` — the environment cannot supply the supervisor.
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    std::fs::write(
        &manifest_path,
        manifest.replacen("[services]\n", "[services]\nweb.command = \"true\"\n", 1),
    )
    .unwrap();

    let sandbox_name = format!("e2ehardsvc{}", std::process::id());
    let (dc_manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\nisolation = \"hardened\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let err = up(&dc_manifest, &project_root, &UpOptions::default())
        .expect_err("hardened `up` must reject declared services with no process-compose");

    // Layer `provider` (exit code 3), the identical verdict the process
    // tier gives — not `backend`, and not a silently healthy sandbox.
    let rendered = err.to_string();
    assert!(
        rendered.starts_with("provider:"),
        "must fail at layer provider, got: {rendered}"
    );
    assert!(
        rendered.contains("process-compose"),
        "the error must name the missing binary, got: {rendered}"
    );

    // Ordering, not just the verdict: the check has to precede `runsc
    // run`, or a sandbox is left running behind a failed `up`.
    assert!(
        !paths.gvisor_runsc_state.exists(),
        "the requirement must be enforced before any sandbox is started"
    );

    let _ = std::fs::remove_dir_all(&project_root);
    let _ = std::fs::remove_dir_all(&paths.root);
}

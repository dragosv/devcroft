//! `up --recreate` (task 4.2): tears down any existing keeper and starts
//! fully fresh. See `tests/lifecycle_up.rs` for why this needs to be an
//! integration test (real `devcroft` binary path via
//! `CARGO_BIN_EXE_devcroft`) and why each such test gets its own file
//! (each `tests/*.rs` is its own process, so the `DEVCROFT_KEEPER_EXE`
//! env var this test sets can't race another test's).

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, health, up};
use std::process::Command;

#[test]
fn recreate_replaces_a_running_keeper_with_a_fresh_one() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if Command::new("flox").arg("--version").output().is_err() {
        eprintln!("skipping: flox not on PATH");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-lifecycle-recreate-e2e-{}",
        std::process::id()
    ));
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

    let sandbox_name = format!("e2erecreate{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );
    let first_pid = match health(&paths).unwrap() {
        devcroft::lifecycle::Health::Healthy(pid) => pid,
        other => panic!("expected Healthy after up, got {other:?}"),
    };

    let recreate_outcome = up(
        &manifest,
        &project_root,
        &UpOptions {
            recreate: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("up --recreate failed: {e}"));
    assert_eq!(recreate_outcome, UpOutcome::Recreated);

    let second_pid = match health(&paths).unwrap() {
        devcroft::lifecycle::Health::Healthy(pid) => pid,
        other => panic!("expected Healthy after recreate, got {other:?}"),
    };
    assert_ne!(
        first_pid, second_pid,
        "--recreate must replace the keeper process, not reuse it"
    );

    devcroft::lifecycle::down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

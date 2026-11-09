//! Auto-up convenience (task 5.3), end to end: the real built binary,
//! starting from a cold sandbox — no prior `up` call at all — unlike
//! `tests/exec_up.rs` and `tests/shell_up.rs`, which both call `up()`
//! directly before touching the CLI. See `tests/lifecycle_up.rs` for why
//! this needs `CARGO_BIN_EXE_devcroft` and why each such test lives in
//! its own file/process.
//!
//! **Neutral surface** (`test-runtime-fixture`): auto-up is the CLI and the
//! lifecycle, not a provider. It used to `flox init` + `flox install
//! coreutils` purely to get past `up`; it now runs on whichever row is
//! selected. The auto-up half needs a row the CLI can resolve
//! (`cli_drivable`); the `--no-up` half refuses before any provider is
//! touched, so it runs on every row.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{Health, StatePaths, down, health};
use std::process::Command;

#[test]
fn exec_brings_up_a_cold_sandbox_automatically() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    for_each_row("autoup", |fx| {
        let project_root = fx.project_root().to_path_buf();
        let sandbox_name = fx.sandbox_name().to_string();
        let paths = StatePaths::new(&sandbox_name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
        let _ = std::fs::remove_file(&paths.lifecycle_lock);
        assert!(
            matches!(health(&paths).unwrap(), Health::None),
            "sandbox must start cold for this test to mean anything"
        );

        if fx.capabilities().cli_drivable && fx.capabilities().external_utils {
            // No `up` call anywhere above: `exec` on a cold sandbox (exec
            // spec's "Auto-up convenience") must bring it up itself before
            // running the command.
            let out = Command::new(devcroft_bin)
                .arg("exec")
                .arg("--")
                .arg("echo")
                .arg("auto-up-marker-hi")
                .current_dir(&project_root)
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                "auto-up-marker-hi",
                "row {}: stderr {:?}",
                fx.name(),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(out.status.code(), Some(0));
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("is not up; starting it"),
                "expected an auto-up notice on stderr, got {:?}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                matches!(health(&paths).unwrap(), Health::Healthy(_)),
                "sandbox must be up after auto-up"
            );

            down(&sandbox_name).unwrap();
            let _ = std::fs::remove_dir_all(&paths.root);
            let _ = std::fs::remove_file(&paths.lifecycle_lock);
        } else {
            eprintln!(
                "row {}: auto-up half skipped (not drivable through the CLI); \
                 --no-up half still runs",
                fx.name()
            );
        }

        // `--no-up` (exec spec: "... unless --no-up is given") opts back
        // out: on a cold sandbox it must fail rather than silently starting
        // one.
        let out = Command::new(devcroft_bin)
            .arg("exec")
            .arg("--no-up")
            .arg("--")
            .arg("echo")
            .arg("should-not-run")
            .current_dir(&project_root)
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(0));
        assert!(
            !String::from_utf8_lossy(&out.stdout).contains("should-not-run"),
            "the command must never have run with --no-up on a cold sandbox"
        );
        assert!(matches!(health(&paths).unwrap(), Health::None));
    });
}

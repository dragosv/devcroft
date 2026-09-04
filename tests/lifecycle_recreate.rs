//! `up --recreate` (task 4.2): tears down any existing keeper and starts
//! fully fresh. See `tests/lifecycle_up.rs` for why this needs to be an
//! integration test (real `devcroft` binary path via
//! `CARGO_BIN_EXE_devcroft`) and why each such test gets its own file
//! (each `tests/*.rs` is its own process, so the `DEVCROFT_KEEPER_EXE`
//! env var this test sets can't race another test's).
//!
//! **Neutral surface** (`test-runtime-fixture`): replacing a keeper has no
//! provider content, so this asserts it against whichever row
//! `DEVCROFT_TEST_PROVIDER` selects rather than building a flox environment
//! to get past `up`. It used to hardcode flox — `flox init` plus a
//! host-capability probe — purely for that.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, health};

#[test]
fn recreate_replaces_a_running_keeper_with_a_fresh_one() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("recreate", |fx| {
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started,
            "row {}",
            fx.name()
        );
        let first_pid = match health(&paths).unwrap() {
            devcroft::lifecycle::Health::Healthy(pid) => pid,
            other => panic!(
                "row {}: expected Healthy after up, got {other:?}",
                fx.name()
            ),
        };

        let recreate_outcome = fx
            .bring_up(&UpOptions {
                recreate: true,
                ..Default::default()
            })
            .unwrap_or_else(|e| panic!("row {}: up --recreate failed: {e}", fx.name()));
        assert_eq!(recreate_outcome, UpOutcome::Recreated, "row {}", fx.name());

        let second_pid = match health(&paths).unwrap() {
            devcroft::lifecycle::Health::Healthy(pid) => pid,
            other => panic!(
                "row {}: expected Healthy after recreate, got {other:?}",
                fx.name()
            ),
        };
        assert_ne!(
            first_pid,
            second_pid,
            "row {}: --recreate must replace the keeper process, not reuse it",
            fx.name()
        );

        devcroft::lifecycle::down(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
    });
}

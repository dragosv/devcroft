//! Regression test for an adversarial-review finding: `up` had no
//! per-sandbox lock, so two concurrent invocations for the same
//! not-yet-running sandbox could both observe `Health::None`, both
//! resolve the provider and compile the policy, and both bind the
//! control socket and spawn a keeper — the second `write_pidfile`
//! silently overwriting the first's record and orphaning its listener
//! and any sessions it had already accepted, with nothing left on disk
//! for a later `down`/`rm` to find.
//!
//! Fixed with an `flock` held for `up`'s entire critical section
//! (`state::acquire_lifecycle_lock`). This test proves the *observable*
//! consequence: two `up`s racing for the same sandbox must never both
//! report `Started` — with the lock serializing them, the second to
//! acquire it always finds the first's keeper already healthy and
//! reports `AlreadyUp` instead.
//!
//! Both calls run as real concurrent threads inside one test process,
//! genuinely racing for the same lock rather than merely running one after
//! another.
//!
//! **Neutral surface** (`test-runtime-fixture`): the lock is lifecycle
//! code, not provider code. It used to build a flox environment purely to
//! get past `up`; the row now supplies whatever environment it is.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::sync::Barrier;

#[test]
fn two_concurrent_up_calls_for_the_same_sandbox_never_both_start_a_keeper() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("concurrent", |fx| {
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
        let _ = std::fs::remove_file(&paths.lifecycle_lock);

        // A barrier, not just "spawn both threads immediately": makes both
        // threads call `up` at as close to the same instant as possible,
        // maximizing the chance either would have won the race the lock now
        // prevents — spawning without one lets the OS schedule them far
        // enough apart that the bug this test exists to catch could hide.
        let barrier = Barrier::new(2);
        let row: &dyn common::ProviderFixture = fx;
        let outcomes: Vec<UpOutcome> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        row.bring_up(&UpOptions::default())
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap()
                        .unwrap_or_else(|e| panic!("row {}: up failed: {e}", row.name()))
                })
                .collect()
        });

        let started_count = outcomes
            .iter()
            .filter(|o| **o == UpOutcome::Started)
            .count();
        assert_eq!(
            started_count,
            1,
            "row {}: exactly one of the two racing `up` calls must report Started \
             (the other must see the first's keeper and report AlreadyUp); \
             outcomes were: {outcomes:?}",
            row.name()
        );
        assert!(
            outcomes.contains(&UpOutcome::AlreadyUp),
            "row {}: the loser of the race must report AlreadyUp, not also Started; \
             outcomes were: {outcomes:?}",
            row.name()
        );

        down(fx.sandbox_name()).unwrap();
    });
}

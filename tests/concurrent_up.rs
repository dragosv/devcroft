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
//! Uses `up` directly (not the CLI subprocess) so both calls run as real
//! concurrent threads inside one test process, genuinely racing for the
//! same lock rather than merely running one after another.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::process::Command;
use std::sync::{Arc, Barrier};

#[test]
fn two_concurrent_up_calls_for_the_same_sandbox_never_both_start_a_keeper() {
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

    let project_root =
        std::env::temp_dir().join(format!("devcroft-concurrent-up-e2e-{}", std::process::id()));
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

    let sandbox_name = format!("e2econcurrent{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    // A barrier, not just "spawn both threads immediately": makes both
    // threads call `up` at as close to the same instant as possible,
    // maximizing the chance either would have won the race the lock now
    // prevents — spawning without one lets the OS schedule them far
    // enough apart that the bug this test exists to catch could hide.
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let manifest = manifest.clone();
            let project_root = project_root.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                up(&manifest, &project_root, &UpOptions::default())
            })
        })
        .collect();

    let outcomes: Vec<UpOutcome> = handles
        .into_iter()
        .map(|h| {
            h.join()
                .unwrap()
                .unwrap_or_else(|e| panic!("up failed: {e}"))
        })
        .collect();

    let started_count = outcomes
        .iter()
        .filter(|o| **o == UpOutcome::Started)
        .count();
    assert_eq!(
        started_count, 1,
        "exactly one of the two racing `up` calls must report Started \
         (the other must see the first's keeper and report AlreadyUp); \
         outcomes were: {outcomes:?}"
    );
    assert!(
        outcomes.contains(&UpOutcome::AlreadyUp),
        "the loser of the race must report AlreadyUp, not also Started; outcomes were: {outcomes:?}"
    );

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

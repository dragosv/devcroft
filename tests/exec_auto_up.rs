//! Auto-up convenience (task 5.3), end to end: the real built binary,
//! starting from a cold sandbox — no prior `up` call at all — unlike
//! `tests/exec_up.rs` and `tests/shell_up.rs`, which both call `up()`
//! directly before touching the CLI. See `tests/lifecycle_up.rs` for why
//! this needs `CARGO_BIN_EXE_devcroft` and why each such test lives in
//! its own file/process.

use devcroft::lifecycle::{Health, StatePaths, down, health};
use std::process::Command;

fn skip_if_tooling_missing() -> bool {
    !devcroft::policy::backend_supported()
        || Command::new("flox").arg("--version").output().is_err()
}

#[test]
fn exec_brings_up_a_cold_sandbox_automatically() {
    if skip_if_tooling_missing() {
        eprintln!("skipping: flox not on PATH");
        return;
    }

    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-auto-up-e2e-{}", std::process::id()));
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
    // own-policy-baseline excludes host toolchain access, so a bare
    // `flox init` leaves nothing for `exec -- echo` to run.
    let install = Command::new("flox")
        .args(["install", "coreutils"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install coreutils failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        return;
    }

    let sandbox_name = format!("e2eautoup{}", std::process::id());
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!("[sandbox]\nname = {sandbox_name:?}\n"),
    )
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    assert!(
        matches!(health(&paths).unwrap(), Health::None),
        "sandbox must start cold for this test to mean anything"
    );

    // No `up` call anywhere above: `exec` on a cold sandbox (exec spec's
    // "Auto-up convenience") must bring it up itself before running the
    // command.
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
        "auto-up-marker-hi"
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

    // `--no-up` (exec spec: "... unless --no-up is given") opts back out:
    // on a cold sandbox it must fail rather than silently starting one.
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

    let _ = std::fs::remove_dir_all(&project_root);
}

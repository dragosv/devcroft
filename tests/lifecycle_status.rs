//! `status`/`logs`/`ps` (task 4.3), against a real keeper spawned by `up`.
//! See `tests/lifecycle_up.rs` for why this needs to be an integration
//! test (real `devcroft` binary path via `CARGO_BIN_EXE_devcroft`) and why
//! each such test lives in its own file (own process, no shared-env races).

use devcroft::config::parse;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::lifecycle::{
    KeeperStatus, StatePaths, UpOptions, UpOutcome, down, logs, ps, status, up,
};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::process::Command;

#[test]
fn status_logs_and_ps_reflect_a_real_running_keeper() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if Command::new("flox").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-lifecycle-status-e2e-{}",
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
    // own-policy-baseline excludes host toolchain access, so a bare
    // `flox init` leaves nothing for a spawned `sleep` session to run.
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

    let sandbox_name = format!("e2estatus{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let before = status(&manifest).unwrap_or_else(|e| panic!("status failed: {e}"));
    assert_eq!(
        before.keeper,
        KeeperStatus::Healthy {
            uptime_secs: before_uptime(&before),
            session_count: 0
        }
    );
    assert_eq!(
        before.env_stale,
        Some(false),
        "environment just resolved by up must not read as stale"
    );
    assert!(
        before.degraded.is_empty(),
        "a minimal manifest requests nothing degradable"
    );

    // Spawn a long-running session and deliberately leave it connected,
    // so status/ps see it as live rather than already reaped.
    let mut session_client = UnixStream::connect(&paths.socket).unwrap();
    protocol::write_frame(
        &mut session_client,
        &Frame::Spawn(SpawnRequest {
            cmd: "sleep".to_string(),
            args: vec!["100".to_string()],
            cwd: project_root.to_str().unwrap().to_string(),
            env: BTreeMap::new(),
            pty: None,
        }),
    )
    .unwrap();
    match protocol::read_frame(&mut session_client).unwrap() {
        Frame::SpawnOk { .. } => {}
        other => panic!("expected SpawnOk, got {other:?}"),
    }

    let during = status(&manifest).unwrap();
    assert_eq!(
        during.keeper,
        KeeperStatus::Healthy {
            uptime_secs: before_uptime(&during),
            session_count: 1
        }
    );

    let sandboxes = ps().unwrap();
    let ours = sandboxes
        .iter()
        .find(|s| s.name == sandbox_name)
        .unwrap_or_else(|| panic!("ps did not list {sandbox_name}: {sandboxes:?}"));
    assert_eq!(
        ours.project_root.as_deref(),
        Some(project_root.to_str().unwrap())
    );
    assert!(matches!(
        ours.keeper,
        KeeperStatus::Healthy {
            session_count: 1,
            ..
        }
    ));

    let log_contents = logs(&sandbox_name, None).unwrap();
    assert!(
        log_contents.contains("spawn session=") && log_contents.contains("sleep 100"),
        "expected a spawn record in the log, got: {log_contents:?}"
    );

    drop(session_client);
    down(&sandbox_name).unwrap();
    let after = status(&manifest).unwrap();
    assert_eq!(after.keeper, KeeperStatus::None);

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

/// `uptime_secs` is real wall-clock elapsed time and not otherwise
/// asserted precisely; this just extracts it so `assert_eq!` can compare
/// the rest of the `Healthy` variant structurally.
fn before_uptime(s: &devcroft::lifecycle::SandboxStatus) -> u64 {
    match s.keeper {
        KeeperStatus::Healthy { uptime_secs, .. } => uptime_secs,
        ref other => panic!("expected Healthy, got {other:?}"),
    }
}

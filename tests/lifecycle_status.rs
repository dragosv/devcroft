//! `status`/`logs`/`ps` (task 4.3), against a real keeper spawned by `up`.
//! See `tests/lifecycle_up.rs` for why this needs to be an integration
//! test (real `devcroft` binary path via `CARGO_BIN_EXE_devcroft`) and why
//! each such test lives in its own file (own process, no shared-env races).
//!
//! **Neutral surface** (`test-runtime-fixture`): what a keeper reports about
//! itself is devcroft's own behaviour, so this runs against whichever row is
//! selected. One assertion here *is* provider-shaped — staleness — and is
//! gated on the row's capability rather than on its name.

mod common;

use common::for_each_row;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::lifecycle::{KeeperStatus, StatePaths, UpOptions, UpOutcome, down, logs, ps, status};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;

#[test]
fn status_logs_and_ps_reflect_a_real_running_keeper() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("status", |fx| {
        let manifest = fx.manifest();
        let sandbox_name = fx.sandbox_name().to_string();
        let project_root = fx.project_root().to_path_buf();
        let paths = StatePaths::new(&sandbox_name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
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
        // Gated on the capability, not the row's name: a row whose provider is
        // injected has its fingerprint honoured by `up` and ignored by `status`,
        // which re-derives one from the manifest.
        if fx.capabilities().staleness {
            assert_eq!(
                before.env_stale,
                Some(false),
                "environment just resolved by up must not read as stale"
            );
        }
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
                // Through the closure's own absolute shell, not a bare
                // `"sleep"`: a bare name is resolved by the *sandbox's* `PATH`,
                // whose tail is the host's directories, so it lands on a host
                // binary the policy denies (CLAUDE.md's shell invariant, which
                // applies to any bare command name, not just `sh`). `sleep`
                // itself comes from the coreutils installed above.
                cmd: devcroft::lifecycle::read_meta(&paths.meta)
                    .unwrap()
                    .and_then(|m| m.shell)
                    .unwrap_or_else(|| "sh".to_string()),
                args: vec!["-c".to_string(), "sleep 100".to_string()],
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
    });
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

//! `up` spawns a real keeper that accepts a session, and `down` tears it
//! back down.
//!
//! An integration test rather than a unit one because it needs the real
//! `devcroft` binary path (`CARGO_BIN_EXE_devcroft`), and its own file
//! because each `tests/*.rs` is its own process — so the
//! `DEVCROFT_KEEPER_EXE` it sets cannot race another test's.
//!
//! **Neutral surface** (`test-runtime-fixture`): none of what it asserts —
//! the keeper starts, `up` is idempotent, a session runs and exits 0, `down`
//! removes the socket — is a claim about any provider. It used to build a
//! flox environment (`flox init` + `flox install bash`) purely to get past
//! `up`; now the row supplies whatever environment it is.

mod common;

use common::for_each_row;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::lifecycle::{Health, StatePaths, UpOptions, UpOutcome, down, health};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;

#[test]
fn up_spawns_a_working_keeper_and_down_tears_it_back_down() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("up", |fx| {
        // Clean slate: a leftover dir from a previous crashed run under the
        // same pid (unlikely, but pids do get reused) must not be mistaken
        // for a healthy keeper.
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        let outcome = fx
            .bring_up(&UpOptions::default())
            .unwrap_or_else(|e| panic!("row {}: up failed: {e}", fx.name()));
        assert_eq!(outcome, UpOutcome::Started, "row {}", fx.name());

        // A second `up` against the same healthy keeper is a no-op.
        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::AlreadyUp,
            "row {}",
            fx.name()
        );

        let mut client = UnixStream::connect(&paths.socket).unwrap();
        protocol::write_frame(
            &mut client,
            &Frame::Spawn(SpawnRequest {
                // The closure's own absolute shell, not a bare `"sh"` — a bare
                // name is resolved by the *sandbox's* `PATH`, whose tail is the
                // host's directories, so it lands on a host binary the policy
                // denies (CLAUDE.md's shell invariant).
                cmd: devcroft::lifecycle::read_meta(&paths.meta)
                    .unwrap()
                    .and_then(|m| m.shell)
                    .unwrap_or_else(|| "sh".to_string()),
                args: vec!["-c".to_string(), "echo hello-e2e".to_string()],
                cwd: fx.project_root().to_str().unwrap().to_string(),
                env: BTreeMap::new(),
                pty: None,
            }),
        )
        .unwrap();
        match protocol::read_frame(&mut client).unwrap() {
            Frame::SpawnOk { .. } => {}
            other => panic!("row {}: expected SpawnOk, got {other:?}", fx.name()),
        }
        let mut out = Vec::new();
        loop {
            match protocol::read_frame(&mut client).unwrap() {
                Frame::Stdout(bytes) | Frame::Stderr(bytes) => out.extend_from_slice(&bytes),
                Frame::Exit(status) => {
                    assert_eq!(status.code, Some(0), "row {}", fx.name());
                    break;
                }
                other => panic!("row {}: unexpected frame: {other:?}", fx.name()),
            }
        }
        assert!(
            String::from_utf8_lossy(&out).contains("hello-e2e"),
            "row {}: expected session output to contain the echoed text, got {out:?}",
            fx.name()
        );
        drop(client);

        down(fx.sandbox_name()).unwrap();
        assert_eq!(health(&paths).unwrap(), Health::None, "row {}", fx.name());
        assert!(UnixStream::connect(&paths.socket).is_err());

        let _ = std::fs::remove_dir_all(&paths.root);
    });
}

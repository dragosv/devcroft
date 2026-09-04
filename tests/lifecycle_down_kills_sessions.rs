//! `down` (task 4.2): the lifecycle spec's "Down with live sessions"
//! scenario — an active session must actually die, not just lose its
//! keeper. See `tests/lifecycle_up.rs` for why this is an integration
//! test and why each such test lives in its own file.

mod common;

use common::for_each_row;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn is_alive(pid: libc::pid_t) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn down_terminates_a_live_session_process_not_just_the_keeper() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("downkill", |fx| {
        let sandbox_name = fx.sandbox_name().to_string();
        let project_root = fx.project_root().to_path_buf();
        let paths = StatePaths::new(&sandbox_name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started
        );

        // The absolute shell `up` resolved out of this sandbox's own closure,
        // not a bare `"sh"`. A bare name is resolved by the *sandbox's* `PATH`,
        // whose tail is the host's own directories, so it lands on a host
        // binary the policy denies — CLAUDE.md calls this out as load-bearing
        // for every call site, and this test was one more of them.
        let shell = devcroft::lifecycle::read_meta(&paths.meta)
            .unwrap()
            .and_then(|m| m.shell)
            .unwrap_or_else(|| "sh".to_string());

        let mut client = UnixStream::connect(&paths.socket).unwrap();
        protocol::write_frame(
            &mut client,
            &Frame::Spawn(SpawnRequest {
                cmd: shell,
                // Prints its own pid first so the test can watch it directly
                // (host-visible: design.md decision 5, no pid-namespace
                // separation between sandboxes in MVP).
                args: vec!["-c".to_string(), "echo $$; sleep 100".to_string()],
                cwd: project_root.to_str().unwrap().to_string(),
                env: BTreeMap::new(),
                pty: None,
            }),
        )
        .unwrap();
        match protocol::read_frame(&mut client).unwrap() {
            Frame::SpawnOk { .. } => {}
            other => panic!("expected SpawnOk, got {other:?}"),
        }
        let session_pid: libc::pid_t = loop {
            match protocol::read_frame(&mut client).unwrap() {
                Frame::Stdout(bytes) => {
                    if let Ok(pid) = String::from_utf8_lossy(&bytes).trim().parse() {
                        break pid;
                    }
                }
                other => panic!("unexpected frame before pid line: {other:?}"),
            }
        };
        assert!(is_alive(session_pid), "session process should be running");

        // Deliberately don't close `client` or read its Exit frame first —
        // this stands in for a client that's simply still attached when
        // `down` runs, exercising teardown of a live session, not a
        // disconnect.
        down(&sandbox_name).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while is_alive(session_pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !is_alive(session_pid),
            "down must terminate live sessions, not just the keeper"
        );

        let _ = std::fs::remove_dir_all(&paths.root);
        let _ = std::fs::remove_dir_all(&project_root);
    });
}

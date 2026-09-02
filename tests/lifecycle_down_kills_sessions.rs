//! `down` (task 4.2): the lifecycle spec's "Down with live sessions"
//! scenario — an active session must actually die, not just lose its
//! keeper. See `tests/lifecycle_up.rs` for why this is an integration
//! test and why each such test lives in its own file.

use devcroft::config::parse;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::process::Command;
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
        "devcroft-lifecycle-downkill-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    // Resolved rather than as spelled: the sandbox's policy is compiled
    // from this path, and macOS matches paths as written — `temp_dir()`
    // is `/var/folders/…`, a symlink, so a cwd named that way is denied
    // under a grant built from its target. No-op on Linux, where
    // Landlock works on inodes. See `docs/known-gaps.md`.
    let project_root = project_root.canonicalize().unwrap_or(project_root);
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
    // `flox init` leaves nothing for a spawned session to exec.
    let install = Command::new("flox")
        .args(["install", "bash", "coreutils"])
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

    let sandbox_name = format!("e2edownkill{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let mut client = UnixStream::connect(&paths.socket).unwrap();
    protocol::write_frame(
        &mut client,
        &Frame::Spawn(SpawnRequest {
            cmd: "sh".to_string(),
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
}

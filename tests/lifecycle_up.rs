//! Integration test for `lifecycle::up`/`down` (task 4.2), against the
//! real binaries the devcontainer provides (flox, nono). Lives under
//! `tests/` rather than as a unit test because it needs the actual built
//! `devcroft` binary path (`CARGO_BIN_EXE_devcroft`) to re-exec as the
//! keeper — `std::env::current_exe()` inside a `src/`-embedded unit test
//! resolves to the libtest harness binary instead, which is not
//! `devcroft` and does not understand `__keeper <fd>` as an argument (see
//! `lifecycle::up`'s `keeper_exe` for the override hook this test uses).

use devcroft::config::parse;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::lifecycle::{Health, StatePaths, UpOptions, UpOutcome, down, health, up};
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::process::Command;

#[test]
fn up_spawns_a_working_keeper_and_down_tears_it_back_down() {
    if Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
    {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    // SAFETY: this process runs a single test (integration tests each get
    // their own binary/process), so there is nothing else to race.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root =
        std::env::temp_dir().join(format!("devcroft-lifecycle-up-e2e-{}", std::process::id()));
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

    let sandbox_name = format!("e2etest{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();

    // Clean slate: a leftover dir from a previous crashed run under the
    // same pid (unlikely, but pids do get reused) must not be mistaken
    // for a healthy keeper.
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let outcome = up(&manifest, &project_root, &UpOptions::default())
        .unwrap_or_else(|e| panic!("up failed: {e}"));
    assert_eq!(outcome, UpOutcome::Started);

    // A second `up` against the same healthy keeper is a no-op.
    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::AlreadyUp
    );

    let mut client = UnixStream::connect(&paths.socket).unwrap();
    protocol::write_frame(
        &mut client,
        &Frame::Spawn(SpawnRequest {
            cmd: "sh".to_string(),
            args: vec!["-c".to_string(), "echo hello-e2e".to_string()],
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
    let mut out = Vec::new();
    loop {
        match protocol::read_frame(&mut client).unwrap() {
            Frame::Stdout(bytes) | Frame::Stderr(bytes) => out.extend_from_slice(&bytes),
            Frame::Exit(status) => {
                assert_eq!(status.code, Some(0));
                break;
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    }
    assert!(
        String::from_utf8_lossy(&out).contains("hello-e2e"),
        "expected session output to contain the echoed text, got {out:?}"
    );
    drop(client);

    down(&sandbox_name).unwrap();
    assert_eq!(health(&paths).unwrap(), Health::None);
    assert!(UnixStream::connect(&paths.socket).is_err());

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

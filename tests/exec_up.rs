//! `devcroft exec` (task 5.1), end to end: the real built binary, a real
//! keeper spawned by `up`, real subprocess exit codes and signals. See
//! `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn exec_propagates_exit_code_maps_cwd_and_forwards_sigint() {
    if Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
    {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-exec-up-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    // Canonicalized so the `pwd` comparison below matches what the OS
    // actually reports: on macOS `std::env::temp_dir()` returns a
    // `/var/...` path, but `/var` is itself a symlink to `/private/var`,
    // which `pwd` inside the spawned session resolves.
    let project_root = project_root.canonicalize().unwrap();
    let src_dir = project_root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
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

    let sandbox_name = format!("e2eexec{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // Exit-code propagation (exec spec scenario).
    let out = Command::new(devcroft_bin)
        .arg("exec")
        .arg(&sandbox_name)
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg("echo hi; exit 42")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
    assert_eq!(out.status.code(), Some(42));

    // Working-directory mapping (exec spec scenario): running `exec` from
    // `<root>/src` starts the command there too, resolved via
    // config::discover finding devcroft.toml in an ancestor — so this
    // omits the explicit sandbox name deliberately.
    // `up` above didn't need a devcroft.toml on disk (it took a parsed
    // Manifest directly); `exec`'s name resolution does, since it walks
    // ancestors for the file. Its declared name must match the sandbox
    // `up` already started.
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!("[sandbox]\nname = {sandbox_name:?}\n"),
    )
    .unwrap();
    let out = Command::new(devcroft_bin)
        .arg("exec")
        .arg("--")
        .arg("pwd")
        .current_dir(&src_dir)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        src_dir.to_str().unwrap(),
        "exec from <root>/src must run the command in <root>/src"
    );
    assert_eq!(out.status.code(), Some(0));

    // Signal forwarding (exec spec scenario): Ctrl-C during `exec --
    // sleep 100` reaches the child and devcroft exits 130.
    let mut child = Command::new(devcroft_bin)
        .arg("exec")
        .arg(&sandbox_name)
        .arg("--")
        .arg("sleep")
        .arg("100")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    // Give the client time to connect, spawn the session, and install its
    // signal mask (all near-instant) before signaling it.
    std::thread::sleep(Duration::from_millis(500));
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130));

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

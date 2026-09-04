//! `devcroft exec` (task 5.1), end to end: the real built binary, a real
//! keeper spawned by `up`, real subprocess exit codes and signals. See
//! `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.
//!
//! **Neutral surface** (`test-runtime-fixture`): exit codes, cwd mapping and
//! signal forwarding are devcroft's own behaviour, so this runs against
//! whichever row is selected instead of building a flox environment to get
//! past `up`.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn exec_propagates_exit_code_maps_cwd_and_forwards_sigint() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    for_each_row("exec", |fx| {
        // Gated on the capability, not the row's name: this asserts `exec`
        // of *external commands* (`sh`, `pwd`, `sleep`), so a row that
        // supplies only a shell cannot run it. Rewriting it to go through
        // `sh -c` would change what it tests.
        if !fx.capabilities().external_utils {
            eprintln!(
                "skipping exec on row {}: no external utilities in this row's environment",
                fx.name()
            );
            return;
        }
        let sandbox_name = fx.sandbox_name().to_string();
        let project_root = fx.project_root().to_path_buf();
        // The cwd-mapping assertion runs `exec` from a subdirectory.
        let src_dir = project_root.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let paths = StatePaths::new(&sandbox_name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started
        );

        // Exit-code propagation (exec spec scenario).
        //
        // Run from the project, as a user would: `devcroft exec` passes the
        // caller's own cwd through to the session unchanged (exec spec,
        // "Working directory mapping"), so invoking it from the *crate* root
        // asks the sandbox to start a process in a directory its policy does
        // not grant. It happened to work on some rows and not others, which
        // is the signature of testing an accident.
        let out = Command::new(devcroft_bin)
            .arg("exec")
            .arg(&sandbox_name)
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg("echo hi; exit 42")
            .current_dir(&project_root)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
        assert_eq!(out.status.code(), Some(42));

        // Working-directory mapping (exec spec scenario): running `exec` from
        // `<root>/src` starts the command there too, resolved via
        // config::discover finding devcroft.toml in an ancestor — so this
        // omits the explicit sandbox name deliberately.
        // The row already wrote a `devcroft.toml` naming this sandbox, which is
        // what `exec`'s ancestor walk needs. This used to rewrite it here; doing
        // that now would drop the row's `[env] provider` line.
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
            .current_dir(&project_root)
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
    });
}

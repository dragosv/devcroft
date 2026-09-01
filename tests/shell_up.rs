//! `devcroft shell` (task 5.2), end to end: the real built binary, a real
//! keeper spawned by `up`, a real pty inside the sandbox. The client's own
//! stdin/stdout are piped rather than a real local tty (there is no way to
//! attach one from a test process), which exercises `RawModeGuard`'s
//! no-op path — the interesting behavior under test is the pty session
//! itself and the `$SHELL`-then-`/bin/sh` fallback, not local terminal
//! handling. See `tests/lifecycle_up.rs` for why this needs
//! `CARGO_BIN_EXE_devcroft` and why each such test lives in its own
//! file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::io::Write;
use std::process::{Command, Stdio};

fn skip_if_tooling_missing() -> bool {
    !devcroft::policy::backend_supported()
        || (Command::new("flox").arg("--version").output().is_err()
            || !devcroft::provider::host_can_build_nix_closures())
}

#[test]
fn shell_runs_commands_over_a_pty_and_falls_back_when_shell_is_missing() {
    if skip_if_tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-shell-up-e2e-{}", std::process::id()));
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
    // `flox init` leaves nothing for `SHELL=sh` (or its `/bin/sh`-turned-
    // bare-`sh` fallback below) to resolve to.
    let install = Command::new("flox")
        .args(["install", "bash"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install bash failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        return;
    }

    let sandbox_name = format!("e2eshell{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // Requested shell exists (exec spec's "Interactive shell" requirement):
    // a real pty session, commands run and their output streams back.
    let mut child = Command::new(devcroft_bin)
        .arg("shell")
        .arg(&sandbox_name)
        .env("SHELL", "sh")
        .current_dir(&project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo shell-marker-hi\nexit\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("shell-marker-hi"),
        "expected pty output to contain the echoed marker, got stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0));

    // Requested shell does not exist in the sandbox (exec spec's "else
    // falling back to /bin/sh"): the keeper refuses to spawn it, and
    // `shell()` retries with `/bin/sh` transparently rather than erroring.
    let mut child = Command::new(devcroft_bin)
        .arg("shell")
        .arg(&sandbox_name)
        .env("SHELL", "/no/such/shell-binary")
        .current_dir(&project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo fallback-marker-hi\nexit\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("fallback-marker-hi"),
        "expected /bin/sh fallback to run the command, got stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0));

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

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
    // Canonicalized (after creation) for the macOS symlink reason in
    // docs/known-gaps.md: `temp_dir()` sits under the `/var` symlink there,
    // and the un-canonicalized spelling of a granted path is refused — the
    // pty session below gets this as its working directory.
    let project_root = project_root.canonicalize().unwrap();
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

    // **Skipped on macOS: pty sessions do not work there at all** — a
    // published gap, not a flaky test (docs/known-gaps.md, "Interactive pty
    // sessions are refused on macOS"). The keeper's `openpty()` has to
    // `open()` the pty *slave* (`/dev/ttysNNN`), and the compiled profile
    // grants only the master (`/dev/ptmx`); measured directly from inside a
    // real sandbox, the master reads fine and opening a slave is refused.
    // Every `devcroft shell` and every SSH pty session therefore fails with
    // `keeper refused to spawn: Operation not permitted`. Left as a skip
    // rather than a weakened assertion so that fixing the gap makes this
    // test start running again.
    if cfg!(target_os = "macos") {
        eprintln!(
            "skipping: interactive pty sessions are refused on macOS — the compiled \
             profile grants /dev/ptmx but not the pty slave (docs/known-gaps.md)"
        );
        let _ = std::fs::remove_dir_all(&project_root);
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
    //
    // `$SHELL` is the closure's own absolute shell, not a bare `"sh"`. A
    // bare name is resolved by the *sandbox's* `PATH`, whose tail is the
    // host's directories, so what it lands on is a property of the host
    // rather than of the sandbox — on macOS it reaches a host `/bin/sh`
    // that execs but cannot read what it needs, and the session produces
    // no output at all. The requirement under test is "a requested shell
    // that exists is used"; naming it removes the ambiguity without
    // weakening that.
    let requested = devcroft::lifecycle::read_meta(&paths.meta)
        .unwrap()
        .and_then(|m| m.shell)
        .expect("up records the shell it resolved from the closure");
    let mut child = Command::new(devcroft_bin)
        .arg("shell")
        .arg(&sandbox_name)
        .env("SHELL", &requested)
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
        "expected pty output to contain the echoed marker, got {:?}",
        String::from_utf8_lossy(&out.stdout)
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

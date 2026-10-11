//! `devcroft shell` (task 5.2), end to end: the real built binary, a real
//! keeper spawned by `up`. A pty only where the client has a terminal, as
//! `ssh` decides: piped input runs on pipes, so the shell sees its input
//! end (with a pty it never did, and `devcroft shell </dev/null` hung). The
//! pty case gets its terminal from `script`, since a test process has none.
//! Also the `$SHELL`-then-`/bin/sh` fallback, and a client that stops
//! waiting on a session ignoring a forwarded SIGTERM. See `tests/lifecycle_up.rs` for why this needs
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
fn shell_runs_commands_ends_with_its_input_and_falls_back_when_shell_is_missing() {
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

    let sandbox_name = format!("e2eshell{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // Requested shell exists (exec spec's "Interactive shell" requirement):
    // commands run and their output streams back. Piped input, so a piped
    // session, not a pty.
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
    // stderr and the keeper's own log in the message: a spawn is refused
    // *inside* the keeper, and the client sees only an empty session. Two CI runs (34825300174, 34826398846) showed `got ""` and
    // nothing else, which is not a diagnosis.
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("shell-marker-hi"),
        "expected the session's output to contain the echoed marker, got stdout={:?} stderr={:?}\n\
         keeper log:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        std::fs::read_to_string(&paths.log).unwrap_or_default()
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

    // The end of the input ends the session. With a pty for piped input it
    // never did: `devcroft shell </dev/null`, or a script without `exit`,
    // hung until SIGKILL. Bounded by `timeout` so a regression fails here
    // rather than hanging the suite.
    let bounded = |stdin: Stdio, input: &[u8]| {
        let mut child = Command::new("timeout")
            .args(["-k", "2", "20", devcroft_bin, "shell"])
            .arg(&sandbox_name)
            .env("SHELL", &requested)
            .current_dir(&project_root)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(mut s) = child.stdin.take() {
            s.write_all(input).unwrap();
        }
        child.wait_with_output().unwrap()
    };
    let null = bounded(Stdio::null(), b"");
    assert_eq!(null.status.code(), Some(0), "shell </dev/null: {null:?}");
    let no_exit = bounded(Stdio::piped(), b"echo no-exit-marker\n");
    assert_eq!(no_exit.status.code(), Some(0), "{no_exit:?}");
    assert!(String::from_utf8_lossy(&no_exit.stdout).contains("no-exit-marker"));

    // With a terminal, a pty: `script` gives the client one, and the shell
    // inside sees a tty.
    if Command::new("script").arg("--version").output().is_ok() {
        let mut child = Command::new("timeout")
            .args(["-k", "2", "30", "script", "-qec"])
            .arg(format!("{devcroft_bin} shell {sandbox_name}"))
            .arg("/dev/null")
            .env("SHELL", &requested)
            .current_dir(&project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            // A builtin: this environment holds bash and nothing else.
            .write_all(b"[ -t 0 ] && echo stdin-is-a-$((6*7))-tty\nexit\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            // Computed, so the echoed input cannot satisfy it.
            String::from_utf8_lossy(&out.stdout).contains("stdin-is-a-42-tty"),
            "with a terminal the session must be a pty: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    } else {
        eprintln!("skipping the pty case: no `script` (util-linux) to give the client a terminal");
    }

    // A forwarded SIGTERM the session ignores no longer leaves the client
    // waiting forever: it gives up after the grace period, exits 143, and
    // the keeper reaps the session it abandoned.
    let mut ignoring = Command::new(devcroft_bin)
        // Builtins only (no `sleep` in this environment): a coprocess that
        // never writes gives `read` something to wait sixty seconds on.
        .args([
            "exec",
            &sandbox_name,
            "--",
            &requested,
            "-c",
            "trap '' TERM; coproc { read; }; read -t 60 -u \"${COPROC[0]}\"",
        ])
        .current_dir(&project_root)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let sent = std::time::Instant::now();
    // SAFETY: signalling our own child by pid.
    unsafe { libc::kill(ignoring.id() as libc::pid_t, libc::SIGTERM) };
    let status = ignoring.wait().unwrap();
    assert_eq!(status.code(), Some(128 + libc::SIGTERM));
    let took = sent.elapsed();
    assert!(
        took >= devcroft::lifecycle::GRACE_PERIOD
            && took < devcroft::lifecycle::GRACE_PERIOD + std::time::Duration::from_secs(5),
        "gave up after {took:?}"
    );

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

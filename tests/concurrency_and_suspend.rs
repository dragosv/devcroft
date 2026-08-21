//! Task 7.3: two-sandbox concurrency, and suspend/resume survival —
//! lifecycle spec's "Concurrent sandboxes" and "Suspend/resume survival"
//! requirements. Real `nono`/`flox`, real built binary, same pattern as
//! `tests/cli_lifecycle_and_policy.rs`.
//!
//! Host suspend/resume itself can't be triggered from inside a test (there
//! is no host to suspend), but the property the spec actually asks for is
//! narrower and testable: a keeper process that gets frozen and unfrozen
//! must still work afterward, with the first command after resume verifying
//! health rather than assuming it. SIGSTOP/SIGCONT on the keeper's pid is
//! exactly what a host suspend/resume cycle does to every process in the
//! tree (the freezer mechanism is signal-equivalent from the process's own
//! point of view), so it's used here as the realistic proxy.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn run(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(devcroft_bin())
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

struct Sandbox {
    name: String,
    project_root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str, manifest_extra: &str) -> Option<Self> {
        if Command::new("nono").arg("--version").output().is_err()
            || Command::new("flox").arg("--version").output().is_err()
        {
            eprintln!("skipping: nono and/or flox not on PATH");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }

        let project_root = std::env::temp_dir().join(format!(
            "devcroft-concurrency-suspend-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&project_root);
        std::fs::create_dir_all(&project_root).unwrap();
        // Canonicalized so the `pwd` comparison below matches what the OS
        // actually reports: on macOS `std::env::temp_dir()` returns a
        // `/var/...` path, but `/var` is itself a symlink to
        // `/private/var`, which `pwd` inside the spawned session resolves.
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
            return None;
        }
        // own-policy-baseline excludes host toolchain access, so a bare
        // `flox init` leaves nothing for `exec -- pwd` etc. to run —
        // installing coreutils gives the sandbox its own `pwd`/`echo`.
        let install = Command::new("flox")
            .args(["install", "coreutils"])
            .current_dir(&project_root)
            .output()
            .unwrap();
        if !install.status.success() {
            eprintln!(
                "skipping: flox install coreutils failed: {}",
                String::from_utf8_lossy(&install.stderr)
            );
            return None;
        }

        let name = format!("e2ecs{tag}{}", std::process::id());
        std::fs::write(
            project_root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n{manifest_extra}"),
        )
        .unwrap();

        Some(Sandbox { name, project_root })
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        run(&self.project_root, args)
    }

    fn state_root(&self) -> PathBuf {
        devcroft::lifecycle::StatePaths::new(&self.name)
            .unwrap()
            .root
    }

    fn pid(&self) -> libc::pid_t {
        let paths = devcroft::lifecycle::StatePaths::new(&self.name).unwrap();
        match devcroft::lifecycle::health(&paths).unwrap() {
            devcroft::lifecycle::Health::Healthy(pid) => pid,
            other => panic!("expected a healthy keeper, got {other:?}"),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(self.state_root());
        let _ = std::fs::remove_dir_all(&self.project_root);
    }
}

#[test]
fn two_sandboxes_run_side_by_side_with_disjoint_state_and_policy() {
    let Some(a) = Sandbox::new("a", "\n[network]\ndefault = \"allow\"\n") else {
        return;
    };
    let Some(b) = Sandbox::new("b", "") else {
        return; // default network.default = "deny"
    };

    assert!(a.run(&["up"]).status.success());
    assert!(b.run(&["up"]).status.success());

    // Disjoint state: distinct roots and sockets, both live at once.
    assert_ne!(a.state_root(), b.state_root());
    assert!(a.state_root().exists());
    assert!(b.state_root().exists());

    // Each `exec` reaches its own project root (own environment)...
    let out_a = a.run(&["exec", "--", "pwd"]);
    assert!(out_a.status.success(), "{out_a:?}");
    assert_eq!(
        String::from_utf8_lossy(&out_a.stdout).trim(),
        a.project_root.to_str().unwrap()
    );

    let out_b = b.run(&["exec", "--", "pwd"]);
    assert!(out_b.status.success(), "{out_b:?}");
    assert_eq!(
        String::from_utf8_lossy(&out_b.stdout).trim(),
        b.project_root.to_str().unwrap()
    );

    // ...and its own policy: `a` allows the network by default, `b` denies.
    let why_a = a.run(&["why", "--host", "example.com"]);
    assert!(why_a.status.success(), "{why_a:?}");
    assert!(String::from_utf8_lossy(&why_a.stdout).contains("ALLOWED"));

    let why_b = b.run(&["why", "--host", "example.com"]);
    assert!(why_b.status.success(), "{why_b:?}");
    assert!(String::from_utf8_lossy(&why_b.stdout).contains("DENIED"));

    // `ps` lists both keepers with distinguishable names, independent of
    // each other.
    let ps = run(&a.project_root, &["ps"]);
    assert!(ps.status.success(), "{ps:?}");
    let stdout = String::from_utf8_lossy(&ps.stdout);
    assert!(stdout.contains(&a.name));
    assert!(stdout.contains(&b.name));
}

#[test]
fn keeper_survives_a_freeze_and_the_next_command_verifies_health() {
    let Some(sandbox) = Sandbox::new("freeze", "") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let pid = sandbox.pid();
    unsafe {
        assert_eq!(libc::kill(pid, libc::SIGSTOP), 0, "failed to freeze keeper");
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    unsafe {
        assert_eq!(libc::kill(pid, libc::SIGCONT), 0, "failed to resume keeper");
    }

    // The first command after "resume" must transparently confirm health
    // (via the control-socket probe in `lifecycle::state::health`, which
    // every session-establishing command already goes through) rather than
    // assume it, and behave with no user-visible difference.
    let status = sandbox.run(&["status"]);
    assert!(status.status.success(), "{status:?}");
    assert!(String::from_utf8_lossy(&status.stdout).contains("keeper: healthy"));

    let exec = sandbox.run(&["exec", "--", "echo", "still-alive"]);
    assert!(exec.status.success(), "{exec:?}");
    assert_eq!(String::from_utf8_lossy(&exec.stdout).trim(), "still-alive");
}

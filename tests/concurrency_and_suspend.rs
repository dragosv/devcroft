//! Task 7.3: two-sandbox concurrency, and suspend/resume survival —
//! lifecycle spec's "Concurrent sandboxes" and "Suspend/resume survival"
//! requirements. Real `nono`, real built binary, same pattern as
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

//!
//! **Neutral surface** (`test-runtime-fixture`): neither property is about a
//! provider. Both used to `flox init` + `flox install coreutils` purely to
//! get past `up`; they now run on whichever row is selected. `up` goes
//! through the row (`bring_up`), so a row the CLI cannot resolve still runs;
//! everything after it is the real binary.

mod common;

use common::{ProviderFixture, another_of, for_each_row};
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome};
use std::path::Path;
use std::process::{Command, Stdio};

fn run(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn supported() -> bool {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return false;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    true
}

/// Append to the row's own manifest, keeping its provider and name.
fn append_manifest(fx: &dyn ProviderFixture, extra: &str) {
    let path = fx.project_root().join("devcroft.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}\n{extra}")).unwrap();
}

fn state_root(fx: &dyn ProviderFixture) -> std::path::PathBuf {
    StatePaths::new(fx.sandbox_name()).unwrap().root
}

fn bring_up(fx: &dyn ProviderFixture) {
    assert_eq!(
        fx.bring_up(&UpOptions::default())
            .unwrap_or_else(|e| panic!("row {}: up failed: {e}", fx.name())),
        UpOutcome::Started
    );
}

/// Canonicalized so the `pwd` comparison matches what the OS reports: on
/// macOS `temp_dir()` is under `/var`, a symlink to `/private/var`, which
/// `pwd` inside the session resolves.
fn canonical_root(fx: &dyn ProviderFixture) -> String {
    fx.project_root()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn two_sandboxes_run_side_by_side_with_disjoint_state_and_policy() {
    if !supported() {
        return;
    }
    for_each_row("csa", |a| {
        let b = another_of(a, "csb");
        // Removed on every exit, like the row `for_each_row` owns.
        struct Gone<'a>(&'a dyn ProviderFixture);
        impl Drop for Gone<'_> {
            fn drop(&mut self) {
                let _ = devcroft::lifecycle::rm(self.0.sandbox_name());
                let _ = std::fs::remove_dir_all(self.0.project_root());
            }
        }
        let _b_gone = Gone(b.as_ref());

        append_manifest(a, "[network]\ndefault = \"allow\"\n");
        // `b` keeps the default, network.default = "deny".
        bring_up(a);
        bring_up(b.as_ref());

        // Disjoint state: distinct roots and sockets, both live at once.
        assert_ne!(state_root(a), state_root(b.as_ref()));
        assert!(state_root(a).exists());
        assert!(state_root(b.as_ref()).exists());

        // Each `exec` reaches its own project root (own environment)...
        for fx in [&*a, b.as_ref()] {
            let out = run(fx.project_root(), &["exec", "--", "pwd"]);
            assert!(out.status.success(), "row {}: {out:?}", fx.name());
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                canonical_root(fx)
            );
        }

        // ...and its own policy: `a` allows the network by default, `b`
        // denies.
        let why_a = run(a.project_root(), &["why", "--host", "example.com"]);
        assert!(why_a.status.success(), "{why_a:?}");
        assert!(String::from_utf8_lossy(&why_a.stdout).contains("ALLOWED"));

        let why_b = run(b.project_root(), &["why", "--host", "example.com"]);
        assert!(why_b.status.success(), "{why_b:?}");
        assert!(String::from_utf8_lossy(&why_b.stdout).contains("DENIED"));

        // `ps` lists both keepers with distinguishable names, independent of
        // each other.
        let ps = run(a.project_root(), &["ps"]);
        assert!(ps.status.success(), "{ps:?}");
        let stdout = String::from_utf8_lossy(&ps.stdout);
        assert!(stdout.contains(a.sandbox_name()));
        assert!(stdout.contains(b.sandbox_name()));
    });
}

#[test]
fn keeper_survives_a_freeze_and_the_next_command_verifies_health() {
    if !supported() {
        return;
    }
    for_each_row("freeze", |fx| {
        bring_up(fx);

        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let pid = match devcroft::lifecycle::health(&paths).unwrap() {
            devcroft::lifecycle::Health::Healthy(pid) => pid,
            other => panic!("expected a healthy keeper, got {other:?}"),
        };
        unsafe {
            assert_eq!(libc::kill(pid, libc::SIGSTOP), 0, "failed to freeze keeper");
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        unsafe {
            assert_eq!(libc::kill(pid, libc::SIGCONT), 0, "failed to resume keeper");
        }

        // The first command after "resume" must transparently confirm health
        // (via the control-socket probe in `lifecycle::state::health`, which
        // every session-establishing command already goes through) rather
        // than assume it, and behave with no user-visible difference.
        let status = run(fx.project_root(), &["status"]);
        assert!(status.status.success(), "row {}: {status:?}", fx.name());
        assert!(String::from_utf8_lossy(&status.stdout).contains("keeper: healthy"));

        let exec = run(fx.project_root(), &["exec", "--", "echo", "still-alive"]);
        assert!(exec.status.success(), "{exec:?}");
        assert_eq!(String::from_utf8_lossy(&exec.stdout).trim(), "still-alive");
    });
}

//! `hooks.post_create`/`hooks.post_start` (lifecycle spec's "Hooks run
//! inside the boundary" requirement), end to end against a real keeper
//! under real `nono`/`flox` — not the in-process `Keeper` test double
//! `src/lifecycle/hooks.rs`'s own unit tests use. See
//! `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpError, UpOptions, UpOutcome, down, up};
use std::process::Command;

struct Sandbox {
    name: String,
    project_root: std::path::PathBuf,
    paths: StatePaths,
}

impl Sandbox {
    fn new(tag: &str, manifest_extra: &str) -> Option<Self> {
        if !devcroft::policy::backend_supported() {
            eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
            return None;
        }
        if Command::new("flox").arg("--version").output().is_err() {
            eprintln!("skipping: flox not on PATH");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
        }

        let project_root = std::env::temp_dir().join(format!(
            "devcroft-lifecycle-hooks-{tag}-e2e-{}",
            std::process::id()
        ));
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
            return None;
        }
        // own-policy-baseline excludes host toolchain access, so a bare
        // `flox init` leaves nothing for hooks.rs's `sh -c <cmd>` to run.
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
            return None;
        }

        let name = format!("e2ehook{tag}{}", std::process::id());
        let paths = StatePaths::new(&name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        std::fs::write(
            project_root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n{manifest_extra}"),
        )
        .unwrap();

        Some(Sandbox {
            name,
            project_root,
            paths,
        })
    }

    fn manifest(&self) -> devcroft::config::Manifest {
        let text = std::fs::read_to_string(self.project_root.join("devcroft.toml")).unwrap();
        parse(&text).unwrap().0
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.paths.log).unwrap_or_default()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = down(&self.name);
        let _ = std::fs::remove_dir_all(&self.paths.root);
        let _ = std::fs::remove_dir_all(&self.project_root);
    }
}

#[test]
fn post_create_and_post_start_run_on_first_up_and_appear_in_logs() {
    let Some(sandbox) = Sandbox::new(
        "basic",
        "[hooks]\npost_create = \"echo pc-marker\"\npost_start = \"echo ps-marker\"\n",
    ) else {
        return;
    };
    let manifest = sandbox.manifest();

    let outcome = up(&manifest, &sandbox.project_root, &UpOptions::default())
        .unwrap_or_else(|e| panic!("up failed: {e}"));
    assert_eq!(outcome, UpOutcome::Started);

    let log = sandbox.log();
    assert!(log.contains("pc-marker"), "log was: {log}");
    assert!(log.contains("ps-marker"), "log was: {log}");
}

#[test]
fn post_create_does_not_rerun_on_recovery_but_post_start_does() {
    let Some(sandbox) = Sandbox::new(
        "recover",
        "[hooks]\npost_create = \"echo pc-marker\"\npost_start = \"echo ps-marker\"\n",
    ) else {
        return;
    };
    let manifest = sandbox.manifest();

    assert_eq!(
        up(&manifest, &sandbox.project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );
    let log = sandbox.log();
    assert_eq!(
        log.matches("pc-marker").count(),
        1,
        "post_create must have run exactly once, log was: {log}"
    );
    assert_eq!(sandbox.log().matches("ps-marker").count(), 1);

    // Simulate a crashed keeper (design.md's "Recovery after host reboot"
    // scenario), not a clean `down` — a clean teardown clears state
    // entirely, which would make the next `up` a fresh `Started` again
    // rather than the `Recovered` path this test is actually checking.
    let pid = match devcroft::lifecycle::health(&sandbox.paths).unwrap() {
        devcroft::lifecycle::Health::Healthy(pid) => pid,
        other => panic!("expected a healthy keeper before killing it, got {other:?}"),
    };
    unsafe {
        assert_eq!(libc::kill(pid, libc::SIGKILL), 0);
    }
    // `up` runs in-process here, so *this test process* — not `init` — is
    // the keeper's real parent; unlike a real `devcroft up` invocation
    // (which exits immediately, letting `init` reparent and reap it),
    // nothing reaps this pid on its own, so it would sit as a zombie
    // (still "alive" to `kill(pid, 0)`) for the rest of the test process's
    // life. Reap it explicitly, standing in for `init` the same way
    // `tests/concurrency_and_suspend.rs`'s freeze/resume test's reaper
    // thread does for its own directly-spawned child.
    unsafe {
        libc::waitpid(pid, std::ptr::null_mut(), 0);
    }

    let outcome = up(&manifest, &sandbox.project_root, &UpOptions::default())
        .unwrap_or_else(|e| panic!("recovery up failed: {e}"));
    assert_eq!(outcome, UpOutcome::Recovered);

    // A fresh keeper spawn truncates `paths.log` (`spawn_keeper` opens it
    // via `File::create`), so this is the log from *this* respawn only,
    // not cumulative with the first `up`'s.
    let log = sandbox.log();
    assert!(
        !log.contains("pc-marker"),
        "post_create must not rerun on recovery, log was: {log}"
    );
    assert!(
        log.contains("ps-marker"),
        "post_start must rerun on every keeper start, log was: {log}"
    );
}

#[test]
fn up_recreate_reruns_post_create() {
    let Some(sandbox) = Sandbox::new("recreate", "[hooks]\npost_create = \"echo pc-marker\"\n")
    else {
        return;
    };
    let manifest = sandbox.manifest();

    assert_eq!(
        up(&manifest, &sandbox.project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );
    let log = sandbox.log();
    assert_eq!(
        log.matches("pc-marker").count(),
        1,
        "post_create must have run exactly once, log was: {log}"
    );

    // `--recreate` tears down the old keeper internally
    // (`state::terminate_and_wait`, SIGTERM then SIGKILL after a grace
    // period) before respawning — and hits the same in-process-parent
    // zombie situation `post_create_does_not_rerun_on_recovery_but_post_start_does`
    // documents: since *this test process* is the old keeper's real
    // parent (unlike a real `devcroft up` invocation, reparented to and
    // reaped by `init` once the invoking process exits), nothing reaps it
    // on its own, so `terminate_and_wait`'s liveness poll can't see it
    // die and stalls for the full grace period. Reap it in the
    // background so `--recreate` proceeds promptly, same workaround.
    let old_pid = match devcroft::lifecycle::health(&sandbox.paths).unwrap() {
        devcroft::lifecycle::Health::Healthy(pid) => pid,
        other => panic!("expected a healthy keeper before recreate, got {other:?}"),
    };
    let reaper = std::thread::spawn(move || unsafe {
        libc::waitpid(old_pid, std::ptr::null_mut(), 0);
    });

    let outcome = up(
        &manifest,
        &sandbox.project_root,
        &UpOptions {
            recreate: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("up --recreate failed: {e}"));
    assert_eq!(outcome, UpOutcome::Recreated);
    reaper.join().unwrap();

    // `--recreate` respawns the keeper, which truncates `paths.log`
    // (`spawn_keeper` opens it via `File::create`) — this is the fresh
    // respawn's log, not cumulative with the first `up`'s, so a single
    // occurrence here means `post_create` genuinely reran.
    let log = sandbox.log();
    assert_eq!(
        log.matches("pc-marker").count(),
        1,
        "post_create must have run exactly once, log was: {log}"
    );
}

#[test]
fn a_failing_post_create_hook_fails_up_and_names_the_hook() {
    let Some(sandbox) = Sandbox::new(
        "failing",
        "[hooks]\npost_create = \"exit 5\"\npost_start = \"echo should-not-run\"\n",
    ) else {
        return;
    };
    let manifest = sandbox.manifest();

    let err = up(&manifest, &sandbox.project_root, &UpOptions::default()).unwrap_err();
    assert!(matches!(err, UpError::Keeper(_)));
    assert!(
        err.to_string().contains("post_create"),
        "error must name the failing hook, got: {err}"
    );

    // post_start must not run after post_create fails.
    assert!(!sandbox.log().contains("should-not-run"));
}

#[test]
fn skip_hooks_bypasses_a_failing_hook_entirely() {
    let Some(sandbox) = Sandbox::new("skip", "[hooks]\npost_create = \"exit 5\"\n") else {
        return;
    };
    let manifest = sandbox.manifest();

    let outcome = up(
        &manifest,
        &sandbox.project_root,
        &UpOptions {
            skip_hooks: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("up with --skip-hooks should not fail: {e}"));
    assert_eq!(outcome, UpOutcome::Started);
}

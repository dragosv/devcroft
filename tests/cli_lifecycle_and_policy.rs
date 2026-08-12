//! The remaining CLI surface (task 7.2): `up`/`down`/`rm`/`status`/`logs`/
//! `ps`/`ssh`/`policy`/`why`, through the real built binary against a
//! real `nono`+`flox` sandbox. Non-interactive safety (`--yes` for
//! destructive ops) and the "list known sandboxes on failure" name-
//! resolution behavior are covered here too, since they only make sense
//! to test through the CLI layer that actually implements them (the
//! library functions underneath were already covered by earlier tasks'
//! own test files).

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
    fn new(tag: &str) -> Option<Self> {
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
            "devcroft-cli-lifecycle-{tag}-{}",
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

        let name = format!("e2ecli{tag}{}", std::process::id());
        std::fs::write(
            project_root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n"),
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
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(self.state_root());
        let _ = std::fs::remove_dir_all(&self.project_root);
    }
}

#[test]
fn up_is_idempotent_and_status_ps_reflect_it() {
    let Some(sandbox) = Sandbox::new("up") else {
        return;
    };

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("is started"));

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("already up"),
        "a second `up` must be idempotent"
    );

    let out = sandbox.run(&["status"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("keeper: healthy"));
    assert!(stdout.contains("env: fresh"));

    let out = sandbox.run(&["ps"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains(&sandbox.name));
}

#[test]
fn up_recreate_requires_yes_when_noninteractive() {
    let Some(sandbox) = Sandbox::new("recreate") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let out = sandbox.run(&["up", "--recreate"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "--recreate without --yes must refuse to run non-interactively"
    );
    // Still healthy — the refused --recreate must not have torn anything down.
    assert!(String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("keeper: healthy"));

    let out = sandbox.run(&["up", "--recreate", "--yes"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("recreated"));
}

#[test]
fn down_stops_the_keeper_and_up_starts_it_again() {
    let Some(sandbox) = Sandbox::new("down") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let out = sandbox.run(&["down"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("keeper: not running")
    );

    let out = sandbox.run(&["up"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("keeper: healthy"));
}

#[test]
fn down_rejects_an_unrecognized_flag_instead_of_treating_it_as_a_name() {
    // `down` has no `--yes` (only `rm`/`up --recreate` are destructive
    // enough to need it); passing it anyway must not be silently treated
    // as the positional sandbox name argument.
    let Some(sandbox) = Sandbox::new("downflag") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let out = sandbox.run(&["down", "--yes"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage"),
        "{out:?}"
    );
    // Must not have torn anything down, since it never resolved a name.
    assert!(String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("keeper: healthy"));
}

#[test]
fn rm_requires_yes_when_noninteractive_and_removes_all_state() {
    let Some(sandbox) = Sandbox::new("rm") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());
    let state_root = sandbox.state_root();

    let out = sandbox.run(&["rm"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--yes"),
        "the error should mention --yes"
    );
    assert!(
        state_root.exists(),
        "a refused rm must not have removed any state"
    );

    let out = sandbox.run(&["rm", "--yes"]);
    assert!(out.status.success(), "{out:?}");
    assert!(!state_root.exists());
}

#[test]
fn policy_render_and_why_explain_decisions() {
    let Some(sandbox) = Sandbox::new("policy") else {
        return;
    };

    let out = sandbox.run(&["policy", "--render"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&format!("sandbox: {}", sandbox.name)));
    assert!(stdout.contains("manifest:filesystem.allow"));
    assert!(stdout.contains("baseline"));

    let out = sandbox.run(&["why", "--path", ".", "--op", "read"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ALLOWED"));
    assert!(stdout.contains("manifest:filesystem.allow"));

    // The point of this one: `--path ~/.aws/credentials` arrives here
    // *already expanded* by the shell that ran this test binary's own
    // parent, or — since we pass it via `Command::arg` with no shell
    // involved — expanded by us right here, deliberately, to exercise
    // exactly the real-world case a real shell would produce.
    let home = std::env::var("HOME").unwrap();
    let out = sandbox.run(&[
        "why",
        "--path",
        &format!("{home}/.aws/credentials"),
        "--op",
        "read",
    ]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("DENIED"));
    assert!(
        stdout.contains("baseline"),
        "an absolute path under $HOME must still resolve to the baseline rule \
         devcroft stores as ~/.aws, got {stdout:?}"
    );

    let out = sandbox.run(&["why", "--host", "example.com"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("DENIED"));
}

#[test]
fn logs_show_session_spawn_and_exit_records() {
    let Some(sandbox) = Sandbox::new("logs") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());
    assert!(
        sandbox
            .run(&["exec", "--", "echo", "log-marker"])
            .status
            .success()
    );

    let out = sandbox.run(&["logs", "--tail", "10"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("spawn"));
    assert!(stdout.contains("exit"));
}

#[test]
fn ssh_command_authenticates_through_a_real_ssh_client() {
    if Command::new("ssh").arg("-V").output().is_err() {
        eprintln!("skipping: ssh not on PATH");
        return;
    }
    let Some(sandbox) = Sandbox::new("ssh") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    // No pty (stdin is /dev/null, same as every other test in this file)
    // means the remote shell just sees immediate EOF and exits cleanly —
    // enough to prove the exec-into-real-ssh path authenticates and
    // reaches a shell, without needing a real local pty to drive an
    // interactive session from a test process.
    let out = sandbox.run(&["ssh", "--no-up"]);
    assert!(
        out.status.success(),
        "devcroft ssh must exec a real ssh that authenticates cleanly: {out:?}"
    );
}

#[test]
fn unresolvable_name_lists_known_sandboxes() {
    let Some(sandbox) = Sandbox::new("known") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    let unrelated_dir = std::env::temp_dir().join(format!(
        "devcroft-cli-lifecycle-unrelated-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&unrelated_dir);
    std::fs::create_dir_all(&unrelated_dir).unwrap();

    let out = run(&unrelated_dir, &["exec", "--", "echo", "hi"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Known sandboxes:"));
    assert!(stderr.contains(&sandbox.name));

    let _ = std::fs::remove_dir_all(&unrelated_dir);
}

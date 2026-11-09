//! The remaining CLI surface (task 7.2): `up`/`down`/`rm`/`status`/`logs`/
//! `ps`/`ssh`/`policy`/`why`, through the real built binary against a
//! real `nono` sandbox. Non-interactive safety (`--yes` for
//! destructive ops) and the "list known sandboxes on failure" name-
//! resolution behavior are covered here too, since they only make sense
//! to test through the CLI layer that actually implements them (the
//! library functions underneath were already covered by earlier tasks'
//! own test files).
//!
//! **Neutral surface** (`test-runtime-fixture`): every command here is the
//! CLI's, not a provider's. The tests used to `flox init` + `flox install
//! bash coreutils` purely to get past `up`; they now run on whichever row is
//! selected. The three tests whose subject *is* `devcroft up` need a row the
//! CLI can resolve (`cli_drivable`); the rest bring the sandbox up through
//! the row and drive everything after that through the binary.

mod common;

use common::{ProviderFixture, for_each_row};
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

struct Sandbox<'a> {
    fx: &'a dyn ProviderFixture,
    name: String,
}

impl Sandbox<'_> {
    fn run(&self, args: &[&str]) -> std::process::Output {
        run(self.fx.project_root(), args)
    }

    fn state_root(&self) -> PathBuf {
        devcroft::lifecycle::StatePaths::new(&self.name)
            .unwrap()
            .root
    }

    /// Up through the row: setup for a test whose subject is not `up`.
    fn up(&self) {
        assert_eq!(
            self.fx
                .bring_up(&devcroft::lifecycle::UpOptions::default())
                .unwrap_or_else(|e| panic!("row {}: up failed: {e}", self.fx.name())),
            devcroft::lifecycle::UpOutcome::Started
        );
    }
}

/// Which tests need `devcroft up` itself, rather than a sandbox that is up.
#[derive(PartialEq)]
enum Up {
    ThroughCli,
    Any,
}

fn with_row(tag: &str, up: Up, body: impl Fn(&Sandbox)) {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
    }
    for_each_row(&format!("cli{tag}"), |fx| {
        if up == Up::ThroughCli && !fx.capabilities().cli_drivable {
            eprintln!(
                "skipping row {}: this test's subject is `devcroft up`, and the CLI \
                 cannot resolve this row",
                fx.name()
            );
            return;
        }
        body(&Sandbox {
            fx,
            name: fx.sandbox_name().to_string(),
        });
    });
}

#[test]
fn up_is_idempotent_and_status_ps_reflect_it() {
    with_row("up", Up::ThroughCli, |sandbox| {
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
    });
}

#[test]
fn up_recreate_requires_yes_when_noninteractive() {
    with_row("recreate", Up::ThroughCli, |sandbox| {
        assert!(sandbox.run(&["up"]).status.success());

        let out = sandbox.run(&["up", "--recreate"]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "--recreate without --yes must refuse to run non-interactively"
        );
        // Still healthy — the refused --recreate must not have torn anything down.
        assert!(
            String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("keeper: healthy")
        );

        let out = sandbox.run(&["up", "--recreate", "--yes"]);
        assert!(out.status.success(), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("recreated"));
    });
}

#[test]
fn down_stops_the_keeper_and_up_starts_it_again() {
    with_row("down", Up::ThroughCli, |sandbox| {
        assert!(sandbox.run(&["up"]).status.success());

        let out = sandbox.run(&["down"]);
        assert!(out.status.success(), "{out:?}");
        assert!(
            String::from_utf8_lossy(&sandbox.run(&["status"]).stdout)
                .contains("keeper: not running")
        );

        let out = sandbox.run(&["up"]);
        assert!(out.status.success(), "{out:?}");
        assert!(
            String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("keeper: healthy")
        );
    });
}

#[test]
fn down_rejects_an_unrecognized_flag_instead_of_treating_it_as_a_name() {
    // `down` has no `--yes` (only `rm`/`up --recreate` are destructive
    // enough to need it); passing it anyway must not be silently treated
    // as the positional sandbox name argument.
    with_row("downflag", Up::Any, |sandbox| {
        sandbox.up();

        let out = sandbox.run(&["down", "--yes"]);
        assert_eq!(out.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("usage"),
            "{out:?}"
        );
        // Must not have torn anything down, since it never resolved a name.
        assert!(
            String::from_utf8_lossy(&sandbox.run(&["status"]).stdout).contains("keeper: healthy")
        );
    });
}

#[test]
fn rm_requires_yes_when_noninteractive_and_removes_all_state() {
    with_row("rm", Up::Any, |sandbox| {
        sandbox.up();
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
    });
}

#[test]
fn policy_render_and_why_explain_decisions() {
    with_row("policy", Up::Any, |sandbox| {
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
    });
}

#[test]
fn policy_render_and_why_show_provider_grants_after_up() {
    // Before this fix (add-nix-provider task 3.4), `policy --render` and
    // `why` were pure functions of the manifest and never showed a
    // provider's store grants at all — not even flox's, despite
    // `Origin::Provider` existing since MVP. This proves the CLI now
    // reads back what `up` recorded (`lifecycle::state::Meta`).
    with_row("policygrants", Up::Any, |sandbox| {
        sandbox.up();

        // What the row granted, read back from what `up` recorded rather than
        // spelled per provider: the claim is that `--render` and `why` show it,
        // not that it is `/nix/store`.
        let paths = devcroft::lifecycle::StatePaths::new(&sandbox.name).unwrap();
        let meta = devcroft::lifecycle::read_meta(&paths.meta)
            .unwrap()
            .expect("up records meta");
        let grant = meta
            .read_only_grants
            .first()
            .unwrap_or_else(|| panic!("row {}: up recorded no provider grant", sandbox.fx.name()))
            .clone();

        let out = sandbox.run(&["policy", "--render"]);
        assert!(out.status.success(), "{out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout
                .lines()
                .any(|l| l.contains(&grant) && l.contains("provider:")),
            "policy --render must show the provider grant {grant} after `up`, got: {stdout}"
        );

        let probe = format!("{}/anything", grant.trim_end_matches('/'));
        let out = sandbox.run(&["why", "--path", &probe, "--op", "read"]);
        assert!(out.status.success(), "{out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("ALLOWED"), "{stdout}");
        assert!(stdout.contains("provider:"), "{stdout}");
    });
}

#[test]
fn logs_show_session_spawn_and_exit_records() {
    with_row("logs", Up::Any, |sandbox| {
        sandbox.up();
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
    });
}

#[test]
fn ssh_command_authenticates_through_a_real_ssh_client() {
    if Command::new("ssh").arg("-V").output().is_err() {
        eprintln!("skipping: ssh not on PATH");
        return;
    }
    with_row("ssh", Up::Any, |sandbox| {
        sandbox.up();

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
    });
}

#[test]
fn unresolvable_name_lists_known_sandboxes() {
    with_row("known", Up::Any, |sandbox| {
        sandbox.up();

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
    });
}

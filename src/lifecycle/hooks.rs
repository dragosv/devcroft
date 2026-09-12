//! Hooks (lifecycle spec's "Hooks run inside the boundary" requirement):
//! `hooks.post_create` runs once, after the sandbox's first successful
//! `up` (or after `--recreate`); `hooks.post_start` runs on every keeper
//! start. A provider's own activation script runs ahead of both.
//!
//! All of them run **inside the keeper**, before it starts any declared
//! service and before it accepts a single connection
//! (`fix-service-hook-ordering`). They used to be spawned by `up` over
//! the keeper's control socket, which put them after services — or
//! rather, ordered them against services not at all: the two raced.
//!
//! Wherever they run, they are subject to the full filesystem/network
//! policy and never get provisioning privileges. CLAUDE.md's two-phase
//! execution model draws the line at hooks being project code rather
//! than trusted host-side provisioning, and moving them into the keeper
//! moved them further inside that line, not out of it — the keeper has
//! already applied the compiled profile to itself by the time
//! `run_in_keeper` is called.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::Path;

use crate::keeper::protocol::SpawnRequest;

#[derive(Debug)]
pub enum HookError {
    Connect(io::Error),
    Protocol(io::Error),
    /// The hook ran and exited non-zero or died by signal — named so
    /// `up`'s error names which hook failed, per the spec's "the error
    /// names the hook" scenario.
    Failed {
        name: &'static str,
        detail: String,
    },
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookError::Connect(e) => write!(f, "connecting to keeper: {e}"),
            HookError::Protocol(e) => write!(f, "protocol error: {e}"),
            HookError::Failed { name, detail } => write!(f, "hook `{name}` failed: {detail}"),
        }
    }
}

impl std::error::Error for HookError {}

/// The line the keeper prints, to its own stdout, when a hook fails —
/// and the whole failure channel for [`run_in_keeper`].
///
/// **There is no other channel, and that is forced rather than chosen.**
/// The state directory is baseline-*denied* to the keeper
/// (`policy::DEVCROFT_DATA_DIR`), which is why `up` hands the SSH key
/// material down directly instead of letting the keeper read it off
/// disk. So the keeper cannot write a status file there for `up` to
/// pick up. What it *can* write is its own stdout, because `up` opened
/// the log host-side and passed it as the child's `stdout`/`stderr`
/// (`up.rs`, `spawn_keeper`) — an inherited fd, never a path the keeper
/// opens. `up` is unrestricted host-side code and reads that file
/// normally.
pub const KEEPER_HOOK_FAILURE_PREFIX: &str = "devcroft-hook-failure: ";

/// Runs hooks **inside the keeper**, in order, before it starts any
/// declared service (`fix-service-hook-ordering`, design.md Decision 1
/// shape A).
///
/// Each `(name, command)` runs to completion before the next begins, and
/// the first failure stops the rest — a `post_start` against a project
/// whose `post_create` failed is meaningless, which is the same reason
/// the host-side runner stopped at the first failure when `up` owned
/// this.
///
/// **Why the keeper does not simply call the old host-side runner.**
/// That one reached the keeper *through the control socket*, which means
/// the keeper must already be accepting connections — and a keeper that
/// accepts connections is a keeper that came up. Ordering hooks before
/// services requires running them before the accept loop, so they cannot
/// travel over the socket and are spawned through the backend directly,
/// exactly as services are.
///
/// Output goes to this process's stdout, which is the sandbox log, so
/// `devcroft logs` shows hook output in the same place and the same
/// format it did when `up` streamed it over a session.
pub fn run_in_keeper(
    backend: &dyn crate::keeper::SessionBackend,
    shell: &str,
    project_root: &Path,
    hooks: &[(&'static str, String)],
) -> Result<(), HookError> {
    for (name, cmd) in hooks {
        run_one_in_keeper(backend, shell, project_root, name, cmd)?;
    }
    Ok(())
}

fn run_one_in_keeper(
    backend: &dyn crate::keeper::SessionBackend,
    shell: &str,
    project_root: &Path,
    name: &'static str,
    cmd: &str,
) -> Result<(), HookError> {
    // Same record `run_one` writes, so a reader of `devcroft logs` cannot
    // tell which process ran the hook — only that it ran.
    println!("[hook {name}] $ {cmd}");
    let _ = io::Write::flush(&mut io::stdout());

    let mut spawned = backend
        .spawn(&SpawnRequest {
            cmd: shell.to_string(),
            args: vec!["-c".to_string(), cmd.to_string()],
            cwd: project_root.to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            pty: None,
        })
        .map_err(|e| HookError::Failed {
            name,
            detail: e.to_string(),
        })?;

    // Drain both pipes before waiting. A hook that writes more than a
    // pipe buffer would otherwise block on write while we block on
    // `wait`, and the keeper would never reach its accept loop — a
    // deadlock that only shows up on chatty hooks, which is the worst
    // kind to ship.
    let mut out = std::mem::replace(&mut spawned.stdout, Box::new(io::empty()));
    let err = spawned.stderr.take();
    let pump = err.map(|mut err| {
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = io::Read::read_to_end(&mut err, &mut sink);
            sink
        })
    });
    let mut buf = Vec::new();
    let _ = io::Read::read_to_end(&mut out, &mut buf);
    if let Some(pump) = pump
        && let Ok(mut errbuf) = pump.join()
    {
        buf.append(&mut errbuf);
    }
    let _ = io::Write::write_all(&mut io::stdout(), &buf);
    let _ = io::Write::flush(&mut io::stdout());

    let status = spawned.child.wait().map_err(|e| HookError::Failed {
        name,
        detail: e.to_string(),
    })?;

    if status.success() {
        return Ok(());
    }
    let detail = match (
        std::os::unix::process::ExitStatusExt::signal(&status),
        status.code(),
    ) {
        (Some(sig), _) => format!("killed by signal {sig}"),
        (None, Some(code)) => format!("exited with status {code}"),
        (None, None) => "exited with unknown status".to_string(),
    };
    Err(HookError::Failed { name, detail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keeper::LocalSessionBackend;

    /// A directory the hooks can write markers into. No keeper, no
    /// socket, no listener: [`run_in_keeper`] spawns through the backend
    /// directly, precisely because it has to run *before* the accept loop
    /// exists — so the old harness that stood a `Keeper` up to serve
    /// these is not only unnecessary, it would test the wrong path.
    fn tempdir(tag: &str) -> std::path::PathBuf {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tag.hash(&mut hasher);
        let dir = std::env::temp_dir().join(format!(
            "dchk-{:08x}-{}",
            hasher.finish() as u32,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(dir: &std::path::Path, hooks: &[(&'static str, String)]) -> Result<(), HookError> {
        run_in_keeper(&LocalSessionBackend, "/bin/sh", dir, hooks)
    }

    #[test]
    fn every_hook_runs_and_in_the_order_given() {
        let dir = tempdir("order");
        run(
            &dir,
            &[
                ("activation", "echo activation >> order".to_string()),
                ("post_create", "echo post_create >> order".to_string()),
                ("post_start", "echo post_start >> order".to_string()),
            ],
        )
        .unwrap();

        // The order is asserted, not just the membership. The provider's
        // activation script prepares the environment the manifest's hooks
        // then use, so running them in any other order is a different
        // (and wrong) contract.
        assert_eq!(
            std::fs::read_to_string(dir.join("order")).unwrap(),
            "activation\npost_create\npost_start\n"
        );
    }

    #[test]
    fn each_hook_completes_before_the_next_begins() {
        let dir = tempdir("sequential");
        run(
            &dir,
            &[
                ("activation", "sleep 0.3; echo first >> seq".to_string()),
                ("post_create", "echo second >> seq".to_string()),
            ],
        )
        .unwrap();

        // A slow first hook is the case that matters: the whole defect
        // this replaced was two things started close together and
        // finishing in whichever order the machine felt like.
        assert_eq!(
            std::fs::read_to_string(dir.join("seq")).unwrap(),
            "first\nsecond\n"
        );
    }

    #[test]
    fn no_hooks_is_a_no_op() {
        let dir = tempdir("no-op");
        run(&dir, &[]).unwrap();
        assert!(!dir.join("anything").exists());
    }

    #[test]
    fn a_failing_hook_names_itself_and_stops_the_rest() {
        let dir = tempdir("failing");
        let err = run(
            &dir,
            &[
                ("post_create", "exit 7".to_string()),
                ("post_start", "echo should-not-run >> ran".to_string()),
            ],
        )
        .unwrap_err();

        assert!(matches!(
            err,
            HookError::Failed {
                name: "post_create",
                ..
            }
        ));
        assert!(err.to_string().contains("post_create"));
        assert!(err.to_string().contains("status 7"));
        // Stopping matters as much as failing: a `post_start` run against
        // a project whose `post_create` failed is meaningless.
        assert!(!dir.join("ran").exists());
    }

    #[test]
    fn a_hook_killed_by_a_signal_says_so_rather_than_reporting_a_code() {
        let dir = tempdir("signal");
        let err = run(&dir, &[("activation", "kill -TERM $$".to_string())]).unwrap_err();
        assert!(
            err.to_string().contains("signal"),
            "expected a signal in {err}"
        );
    }

    #[test]
    fn a_chatty_hook_does_not_deadlock() {
        let dir = tempdir("chatty");
        // More than a pipe buffer on both streams. Without draining the
        // pipes before `wait`, the child blocks on write while the keeper
        // blocks on wait, and the sandbox never comes up — a deadlock
        // that only appears on hooks that say a lot.
        run(
            &dir,
            &[(
                "activation",
                "i=0; while [ $i -lt 4000 ]; do echo out-$i; echo err-$i >&2; i=$((i+1)); done"
                    .to_string(),
            )],
        )
        .unwrap();
    }
}

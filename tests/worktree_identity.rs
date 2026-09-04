//! **A sandbox belongs to the project root it was created for**
//! (`add-agent-workload` task group 1).
//!
//! `meta.json` has always recorded `project_root`; nothing compared it. Two
//! git worktrees of one repository share a committed `devcroft.toml`, and so
//! share a sandbox *name* — which meant the second `up` silently adopted the
//! first's keeper. An agent working in worktree B ran against worktree A's
//! environment, its grants, and its services, with nothing said.
//!
//! That is the case this file exercises with a **real** `git worktree`
//! rather than two directories that merely look alike, because the bug is
//! about two checkouts of one repository sharing one committed manifest —
//! reproducing it with unrelated copies would prove something weaker.
//!
//! The fix has two halves and both are asserted here: the refusal, and the
//! `--name` override that makes fan-out possible. Without the second, the
//! first is a refusal with no remedy — the manifest is committed, so a user
//! cannot simply rename one side.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::process::Command;

fn git(args: &[&str], cwd: &std::path::Path) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn two_worktrees_of_one_repo_do_not_silently_share_a_sandbox() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("worktree", |fx| {
        let root = fx.project_root().to_path_buf();

        // Make the row's project a real repository, and commit its manifest —
        // committed is the whole point: that is why both worktrees end up
        // declaring the same sandbox name.
        if !git(&["init", "-q"], &root)
            || !git(&["config", "user.email", "t@example.invalid"], &root)
            || !git(&["config", "user.name", "t"], &root)
            || !git(&["add", "-A"], &root)
            || !git(
                &["-c", "commit.gpgsign=false", "commit", "-qm", "fixture"],
                &root,
            )
        {
            eprintln!(
                "skipping: could not make row {}'s project a git repo",
                fx.name()
            );
            return;
        }

        let second = root.parent().unwrap().join(format!(
            "{}-wt2",
            root.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&second);
        if !git(
            &[
                "worktree",
                "add",
                "-q",
                &second.to_string_lossy(),
                "-b",
                "wt2",
            ],
            &root,
        ) {
            eprintln!("skipping: `git worktree add` failed on row {}", fx.name());
            return;
        }

        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        // First worktree: ordinary `up`.
        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started,
            "row {}",
            fx.name()
        );

        // Second worktree, same committed manifest, same sandbox name. This
        // is the case that used to silently adopt the first's keeper.
        let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
            .arg("up")
            .current_dir(&second)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

        // And the remedy the refusal names must work: same worktree, same
        // committed manifest, its own sandbox. Asserted rather than assumed,
        // because a refusal whose suggested fix does not work is worse than
        // the silence it replaced.
        // Gated on the capability: creating a sandbox through the CLI needs
        // a provider the CLI can resolve from the manifest, which a row whose
        // provider exists only in-process does not have. The *refusal* above
        // is unaffected — the identity check fires before provider
        // resolution, which is why it works on every row.
        let second_named = fx.capabilities().cli_drivable.then(|| {
            let alt = format!("{}wt2", fx.sandbox_name());
            let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
                .args(["up", "--name", &alt])
                .current_dir(&second)
                .output()
                .unwrap();
            let _ = down(&alt);
            if let Ok(p) = StatePaths::new(&alt) {
                let _ = std::fs::remove_dir_all(&p.root);
            }
            out
        });

        // Cleaned up before asserting, so a failure does not leak a keeper
        // or a worktree into the next row.
        let cleanup = |name: &str| {
            let _ = down(name);
        };
        cleanup(fx.sandbox_name());
        let _ = std::fs::remove_dir_all(&paths.root);
        let _ = git(
            &["worktree", "remove", "--force", &second.to_string_lossy()],
            &root,
        );
        let _ = std::fs::remove_dir_all(&second);

        assert!(
            !out.status.success(),
            "row {}: a second worktree must not adopt the first's sandbox; \
             it used to, silently. stderr: {stderr}",
            fx.name()
        );
        assert!(
            stderr.contains("different project root"),
            "row {}: the refusal must say what is wrong, got: {stderr}",
            fx.name()
        );
        // Both roots named, so the user can see which is which rather than
        // being told only that something conflicts.
        assert!(
            stderr.contains(&root.display().to_string())
                && stderr.contains(&second.display().to_string()),
            "row {}: the refusal must name both roots, got: {stderr}",
            fx.name()
        );
        if let Some(named) = second_named {
            assert!(
                named.status.success(),
                "row {}: `--name` must actually resolve the conflict, or the \
                 refusal is a dead end. stderr: {}",
                fx.name(),
                String::from_utf8_lossy(&named.stderr)
            );
        }
        assert!(
            stderr.contains("--name"),
            "row {}: the refusal must name the remedy, or it is a dead end — \
             the manifest is committed, so the user cannot rename one side. \
             got: {stderr}",
            fx.name()
        );
    });
}

/// Same `up` twice from the same root stays idempotent.
///
/// The guard that keeps the check above honest: a project-root comparison
/// that fired on a repeated run would have conflated "a different root" with
/// "the same root again", which is the far more common case and would have
/// broken every ordinary `up`.
#[test]
fn a_repeated_up_from_the_same_root_is_still_a_no_op() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("samreroot", |fx| {
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started
        );
        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::AlreadyUp,
            "row {}: the project-root check must not fire on a repeat run",
            fx.name()
        );

        down(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
    });
}

//! Per-agent git workspace isolation (`add-linux-agent-fleet` D7, and
//! task group 4's `workspace-isolation` capability): a single shared bare
//! mirror per upstream repository, plus an independent clone per agent so
//! no agent's git operations can lock or corrupt another's.
//!
//! Worktrees were the obvious shape and the wrong one — they share the
//! object store *and* refs. `index.lock` is per-worktree but
//! `packed-refs` is not, and git takes locks without retrying, so one
//! agent commits and another gets a spurious failure it has to react to.
//! A mirror-plus-clone pair gives each agent its own ref namespace while
//! keeping disk usage close to a worktree's.
//!
//! Built as a standalone primitive, the same shape as [`super::netns`]:
//! no assumption about the eventual supervisor's directory layout —
//! callers pass every path in, and nothing here reads or writes fleet
//! state of its own.

use std::io;
use std::path::Path;
use std::process::Command;

/// Ensures a bare mirror of `upstream` exists at `mirror_dir`, cloning it
/// fresh if one is not already there, and disables automatic GC on it
/// either way — an existing mirror created before this function's own
/// care about GC timing must not be trusted to already have it disabled.
///
/// `upstream` is whatever `git clone` accepts as a source: a local path
/// or a remote URL. This is the only point in an agent's lifecycle that
/// ever needs the real upstream — every agent clone afterward is sourced
/// from `mirror_dir` itself, never from `upstream` again, which is what
/// keeps per-agent clone creation free of any network dependency on the
/// real remote.
pub fn ensure_mirror(upstream: &str, mirror_dir: &Path) -> io::Result<()> {
    if !mirror_dir.join("HEAD").is_file() {
        run_git(
            None,
            &[
                "clone",
                "--bare",
                "--mirror",
                upstream,
                &mirror_dir.to_string_lossy(),
            ],
        )?;
    }
    disable_auto_gc(mirror_dir)
}

/// Creates one agent's independent clone from the shared mirror.
///
/// Sourced from `mirror_dir`, not the real upstream — the resulting
/// clone's `origin` remote points at the mirror, so nothing in the
/// agent's own git config can push to, or even names, the real upstream.
/// This *is* the workspace-isolation spec's "remove or block the
/// upstream remote in agent clones": there is no separate removal step,
/// because the real remote is never configured to begin with. An agent
/// that pushes at all pushes into the shared mirror, which stays under
/// devcroft's control; publishing from the mirror to the real upstream is
/// a separate, not-yet-built integration step, deliberately not implied
/// by this function.
///
/// `--reference` (not a plain local clone) is deliberate: it shares
/// objects via the clone's `objects/info/alternates` file rather than
/// hardlinking them in, which is what keeps disk usage close to a
/// worktree's rather than duplicating the repository per agent. The
/// tradeoff is that this clone's objects genuinely do not exist without
/// the mirror — pruning the mirror while this clone is live can corrupt
/// it, which is exactly the failure [`run_maintenance`]'s active-agent
/// check exists to prevent.
pub fn create_agent_clone(mirror_dir: &Path, dest: &Path) -> io::Result<()> {
    let mirror = mirror_dir.to_string_lossy();
    run_git(
        None,
        &[
            "clone",
            "--reference",
            &mirror,
            &mirror,
            &dest.to_string_lossy(),
        ],
    )?;
    disable_auto_gc(dest)
}

fn disable_auto_gc(repo_dir: &Path) -> io::Result<()> {
    run_git(Some(repo_dir), &["config", "gc.auto", "0"])
}

/// Runs mirror maintenance (`git gc`), but only when `active_agents` is
/// zero. Refuses otherwise, reporting why rather than silently no-oping —
/// per the spec's "Maintenance is attempted with agents running"
/// scenario: an agent clone references the mirror's objects rather than
/// copying them, so a prune here can remove an object a live clone still
/// needs, and the failure would surface inside that agent long after this
/// call had already returned success.
///
/// `active_agents` is supplied by the caller rather than discovered here
/// on purpose: this module owns git mechanics, not fleet membership, and
/// the eventual supervisor is the only thing that can answer "how many
/// agents are live" correctly.
pub fn run_maintenance(mirror_dir: &Path, active_agents: usize) -> io::Result<()> {
    if active_agents > 0 {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!(
                "refusing mirror maintenance: {active_agents} agent(s) still active \
                 and referencing this mirror's objects"
            ),
        ));
    }
    run_git(Some(mirror_dir), &["gc", "--prune=now"])
}

fn run_git(cwd: Option<&Path>, args: &[&str]) -> io::Result<()> {
    let mut cmd = Command::new("git");
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd
        .args(args)
        .output()
        .map_err(|e| io::Error::other(format!("running `git {}`: {e}", args.join(" "))))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "`git {}` exited with {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-workspace-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A small real upstream repository with one commit, standing in for
    /// "the real remote" — every test below treats this as the only thing
    /// `ensure_mirror` is ever pointed at.
    fn seed_upstream() -> std::path::PathBuf {
        let dir = tempdir("upstream");
        run_git(Some(&dir), &["init", "--initial-branch=main"]).unwrap();
        run_git(Some(&dir), &["config", "user.email", "test@example.com"]).unwrap();
        run_git(Some(&dir), &["config", "user.name", "Test"]).unwrap();
        fs::write(dir.join("README.md"), "seed\n").unwrap();
        run_git(Some(&dir), &["add", "README.md"]).unwrap();
        run_git(Some(&dir), &["commit", "-m", "seed"]).unwrap();
        dir
    }

    fn git_config_value(repo_dir: &Path, key: &str) -> Option<String> {
        let output = Command::new("git")
            .current_dir(repo_dir)
            .args(["config", key])
            .output()
            .unwrap();
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn remotes(repo_dir: &Path) -> String {
        let output = Command::new("git")
            .current_dir(repo_dir)
            .args(["remote", "-v"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    #[test]
    fn ensure_mirror_clones_once_and_disables_auto_gc() {
        let upstream = seed_upstream();
        let mirror = tempdir("mirror").join("repo.git");

        ensure_mirror(&upstream.to_string_lossy(), &mirror).unwrap();
        assert!(
            mirror.join("HEAD").is_file(),
            "expected a bare repo at {mirror:?}"
        );
        assert_eq!(git_config_value(&mirror, "gc.auto").as_deref(), Some("0"));

        // Calling again must not error, and must not re-clone (the real
        // upstream having since vanished, say) — an existing mirror is
        // trusted as-is, only its GC setting is (re-)enforced.
        fs::remove_dir_all(&upstream).unwrap();
        ensure_mirror("this-path-does-not-exist", &mirror).unwrap();
        assert_eq!(git_config_value(&mirror, "gc.auto").as_deref(), Some("0"));

        let _ = fs::remove_dir_all(mirror.parent().unwrap());
    }

    #[test]
    fn agent_clone_never_references_the_real_upstream() {
        let upstream = seed_upstream();
        let mirror = tempdir("mirror").join("repo.git");
        ensure_mirror(&upstream.to_string_lossy(), &mirror).unwrap();

        let agent_dir = tempdir("agent");
        create_agent_clone(&mirror, &agent_dir.join("clone")).unwrap();
        let clone_dir = agent_dir.join("clone");

        assert!(
            clone_dir.join("README.md").is_file(),
            "expected the seed commit's tree"
        );
        assert_eq!(
            git_config_value(&clone_dir, "gc.auto").as_deref(),
            Some("0")
        );

        // The whole point of D7: origin is the mirror, and the real
        // upstream's path appears nowhere in this clone's git config.
        let configured_remotes = remotes(&clone_dir);
        assert!(
            configured_remotes.contains(&mirror.to_string_lossy().to_string()),
            "expected origin to point at the mirror, got: {configured_remotes}"
        );
        assert!(
            !configured_remotes.contains(&upstream.to_string_lossy().to_string()),
            "the real upstream must not be a configured remote at all, got: {configured_remotes}"
        );

        let _ = fs::remove_dir_all(&upstream);
        let _ = fs::remove_dir_all(mirror.parent().unwrap());
        let _ = fs::remove_dir_all(&agent_dir);
    }

    #[test]
    fn agent_clone_shares_objects_via_alternates_not_copies() {
        let upstream = seed_upstream();
        let mirror = tempdir("mirror").join("repo.git");
        ensure_mirror(&upstream.to_string_lossy(), &mirror).unwrap();

        let agent_dir = tempdir("agent");
        let clone_dir = agent_dir.join("clone");
        create_agent_clone(&mirror, &clone_dir).unwrap();

        let alternates = clone_dir.join(".git/objects/info/alternates");
        assert!(
            alternates.is_file(),
            "expected `--reference` to record an alternates file, none found"
        );
        let alternates_content = fs::read_to_string(&alternates).unwrap();
        assert!(
            alternates_content.contains(&mirror.to_string_lossy().to_string()),
            "alternates should point at the mirror's objects: {alternates_content}"
        );

        let _ = fs::remove_dir_all(&upstream);
        let _ = fs::remove_dir_all(mirror.parent().unwrap());
        let _ = fs::remove_dir_all(&agent_dir);
    }

    /// The spec's "Concurrent commits" scenario: several agent clones off
    /// the same mirror commit independently, and none observes a lock
    /// failure caused by another's activity, because each has its own
    /// `packed-refs`/`index.lock` — exactly what the mirror-plus-clone
    /// shape buys over sharing one worktree.
    #[test]
    fn concurrent_commits_across_agent_clones_do_not_collide() {
        let upstream = seed_upstream();
        let mirror = tempdir("mirror").join("repo.git");
        ensure_mirror(&upstream.to_string_lossy(), &mirror).unwrap();

        let agents_dir = tempdir("agents");
        let clones: Vec<_> = (0..4)
            .map(|i| {
                let dest = agents_dir.join(format!("agent-{i}"));
                create_agent_clone(&mirror, &dest).unwrap();
                dest
            })
            .collect();

        let handles: Vec<_> = clones
            .into_iter()
            .enumerate()
            .map(|(i, clone_dir)| {
                std::thread::spawn(move || {
                    run_git(Some(&clone_dir), &["config", "user.email", "a@example.com"]).unwrap();
                    run_git(Some(&clone_dir), &["config", "user.name", "Agent"]).unwrap();
                    fs::write(clone_dir.join(format!("agent-{i}.txt")), "work\n").unwrap();
                    run_git(Some(&clone_dir), &["add", "."]).unwrap();
                    run_git(Some(&clone_dir), &["commit", "-m", &format!("agent {i}")]).unwrap();
                })
            })
            .collect();

        for h in handles {
            h.join()
                .expect("no agent's commit should panic or lock-fail");
        }

        let _ = fs::remove_dir_all(&upstream);
        let _ = fs::remove_dir_all(mirror.parent().unwrap());
        let _ = fs::remove_dir_all(&agents_dir);
    }

    #[test]
    fn maintenance_refuses_while_agents_are_active() {
        let upstream = seed_upstream();
        let mirror = tempdir("mirror").join("repo.git");
        ensure_mirror(&upstream.to_string_lossy(), &mirror).unwrap();

        let err = run_maintenance(&mirror, 2).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        assert!(
            err.to_string().contains('2'),
            "the refusal should name how many agents are active: {err}"
        );

        let _ = fs::remove_dir_all(&upstream);
        let _ = fs::remove_dir_all(mirror.parent().unwrap());
    }

    #[test]
    fn maintenance_runs_when_no_agents_are_active() {
        let upstream = seed_upstream();
        let mirror = tempdir("mirror").join("repo.git");
        ensure_mirror(&upstream.to_string_lossy(), &mirror).unwrap();

        run_maintenance(&mirror, 0).unwrap();

        let _ = fs::remove_dir_all(&upstream);
        let _ = fs::remove_dir_all(mirror.parent().unwrap());
    }
}

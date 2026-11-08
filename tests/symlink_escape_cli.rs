//! Regression test for a symlink-escape bug found by adversarial review:
//! a project-relative `filesystem.allow` entry that was itself a symlink
//! to a directory outside the project root — e.g. `credential-link ->
//! ~/.ssh` — passed every lexical check (the sensitive-path warning, the
//! baseline-deny-unless-granted rule) looking like an ordinary in-project
//! grant, then `nono::allow_path` canonicalized and granted the real
//! target. Fixed by canonicalizing project-relative grants in
//! `policy::capability_set::grant` and rejecting an escape there,
//! checked from both `up` (the actual enforcement path) and
//! `policy --render` (which must not show a policy as fine when it would
//! actually fail to compile).
//!
//! Runs through the real binary against a real symlink on disk, not just
//! the unit tests in `capability_set.rs` — the bug was in what a real
//! filesystem call (`canonicalize`) revealed that a lexical check on the
//! manifest string could not.
//!
//! The `up` half is on the **neutral surface** (`test-runtime-fixture`):
//! the escape is refused while compiling the policy, whatever the
//! provider. It used to `flox init` a project purely to get past provider
//! resolution; it now runs on whichever row is selected, and calls `up`
//! in-process rather than through the binary -- an injected row is not
//! reachable from the CLI. `policy --render` still goes through the binary.

mod common;

use common::for_each_row;
use std::process::Command;

/// A project containing a symlink whose real target sits *outside* the
/// project root — the exact shape the bug needed.
struct EscapeProject {
    project_root: std::path::PathBuf,
    outside_target: std::path::PathBuf,
}

impl EscapeProject {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "devcroft-symlink-escape-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let project_root = base.join("project");
        let outside_target = base.join("outside-secret");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&outside_target).unwrap();
        std::fs::write(outside_target.join("secret.txt"), "do not grant").unwrap();
        std::os::unix::fs::symlink(&outside_target, project_root.join("escape-link")).unwrap();
        std::fs::write(
            project_root.join("devcroft.toml"),
            // Pid-scoped, like every other e2e sandbox name here. It used to
            // be a bare literal while the project root varied by pid, so a
            // leftover state dir from an earlier run belonged to a different
            // root — which `up` silently adopted before
            // `add-agent-workload`'s identity check, and now correctly
            // refuses. The test was relying on the bug.
            format!(
                "[sandbox]\nname = \"symlinkescape{}\"\n\n[filesystem]\nallow = [\".\", \"escape-link\"]\n",
                std::process::id()
            ),
        )
        .unwrap();
        EscapeProject {
            project_root,
            outside_target,
        }
    }
}

impl Drop for EscapeProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project_root);
        let _ = std::fs::remove_dir_all(&self.outside_target);
    }
}

#[test]
fn policy_render_refuses_a_manifest_whose_grant_symlinks_outside_the_project() {
    let project = EscapeProject::new("render");

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .current_dir(&project.project_root)
        .args(["policy", "--render"])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "rendering a policy that cannot actually be enforced must fail, not succeed silently"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("escape-link") && stderr.contains("outside the project root"),
        "stderr should name the entry and explain the escape, got: {stderr}"
    );
}

#[test]
fn up_refuses_a_manifest_whose_grant_symlinks_outside_the_project() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("symlinkescape", |fx| {
        let root = fx.project_root().to_path_buf();
        let outside = root.with_extension("outside-secret");
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "do not grant").unwrap();
        let link = root.join("escape-link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        // Appended to the row's own manifest, so the row keeps whatever
        // else it declares (its provider, its sandbox name).
        let manifest = root.join("devcroft.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            format!("{text}\n[filesystem]\nallow = [\".\", \"escape-link\"]\n"),
        )
        .unwrap();

        let result = fx.bring_up(&devcroft::lifecycle::UpOptions::default());
        // Refused, so normally there is nothing to take down; `rm` keeps a
        // regression that started a sandbox from leaking one.
        let _ = devcroft::lifecycle::rm(fx.sandbox_name());
        let _ = std::fs::remove_dir_all(&outside);

        let err = match result {
            Ok(outcome) => panic!(
                "row {}: up must refuse a grant that symlinks outside the project, got {outcome:?}",
                fx.name()
            ),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("escape-link") && err.contains("outside the project root"),
            "row {}: the error should name the entry and explain the escape, got: {err}",
            fx.name()
        );
    });
}

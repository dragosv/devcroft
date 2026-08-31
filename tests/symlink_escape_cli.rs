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
            "[sandbox]\nname = \"symlinkescape\"\n\n[filesystem]\nallow = [\".\", \"escape-link\"]\n",
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
    let project = EscapeProject::new("up");
    // `up` validates the compiled policy before doing anything provider-
    // related fails first — needs a real flox environment to get past
    // that layer and actually reach the policy-compile check this test
    // is about.
    let flox_ok = Command::new("flox")
        .arg("init")
        .current_dir(&project.project_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    // `flox init` is not the capability this needs: it writes a manifest
    // without building anything, and succeeds on a host whose store is
    // unreachable. `up` then fails at layer `provider` before it ever
    // compiles the policy, and this test reports that as "the escape was
    // not refused" — a false accusation against the code it covers.
    if !flox_ok || !devcroft::provider::host_can_build_nix_closures() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .current_dir(&project.project_root)
        .arg("up")
        .output()
        .unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("escape-link") && stderr.contains("outside the project root"),
        "stderr should name the entry and explain the escape, got: {stderr}"
    );
}

//! Regression test for an adversarial-review finding: `filesystem.allow`
//! granting devcroft's own data dir verbatim (`~/.local/share/devcroft`,
//! the directory holding the ephemeral SSH host key and other runtime
//! state) compiled successfully and produced a real read-write Landlock
//! grant, despite `policy::compile` pushing that exact path into
//! `filesystem_deny` unconditionally and both `policy --render` and `why`
//! reporting it DENIED. The two checks each assumed the other would catch
//! it: `policy::compile`'s doc comment already says the data dir is
//! "never overridable by the manifest", but `check_no_deny_overlaps_allow`
//! specifically excluded an *exact* match between a deny and an allow
//! entry, which is the only way this particular deny (pushed
//! unconditionally, not conditionally like the credential-directory
//! entries) could ever collide with an allow entry at all.
//!
//! Runs through the real binary against a real manifest, not just the
//! unit test in `capability_set.rs` — the bug was specifically that
//! inspection (`policy --render`, `why`) and enforcement disagreed, which
//! only a real end-to-end run against the actual CLI surface proves.

use std::process::Command;

fn scratch_project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "devcroft-data-dir-grant-cli-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn policy_render_refuses_a_manifest_that_grants_the_data_dir_verbatim() {
    let dir = scratch_project("render");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"datadirgrant\"\n\n[filesystem]\nallow = [\".\", \"~/.local/share/devcroft\"]\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .current_dir(&dir)
        .args(["policy", "--render"])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "rendering a policy that grants devcroft's own data dir must fail, not succeed silently"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("~/.local/share/devcroft"),
        "stderr should name the offending entry, got: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

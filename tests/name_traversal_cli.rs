//! Regression test for a path-traversal bug found by adversarial review:
//! `devcroft rm ../VICTIM --yes` recursively deleted a directory outside
//! `~/.local/share/devcroft` entirely, because the explicit name argument
//! reached `StatePaths::new` (and from there `remove_dir_all`) without
//! ever being validated as a sandbox name. Fixed at the choke point
//! (`StatePaths::new` itself, plus `resolve_name_arg` for a clean exit-2
//! message on `down`/`rm` specifically) rather than at each call site, so
//! this test exercises the real binary end to end rather than the fix's
//! internals — the bug was in what survived to a real `remove_dir_all`,
//! not in any one function's logic.

use std::process::Command;

/// A fake `$HOME` with a decoy directory *outside* where devcroft's own
/// state lives, so a passing test proves the decoy survives untouched —
/// the same shape as the live proof used to find this bug.
struct FakeHome {
    root: std::path::PathBuf,
    victim: std::path::PathBuf,
}

impl FakeHome {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "devcroft-name-traversal-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".local/share/devcroft")).unwrap();
        let victim = root.join(".local/share/victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("precious.txt"), "do not delete").unwrap();
        FakeHome { root, victim }
    }
}

impl Drop for FakeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn rm_rejects_a_traversal_name_instead_of_deleting_outside_the_state_root() {
    let home = FakeHome::new("rm");

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .env("HOME", &home.root)
        .args(["rm", "../victim", "--yes"])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "a traversal name must be rejected, not accepted"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not a valid sandbox name"),
        "stderr should name the actual problem, got: {stderr}"
    );
    assert!(
        home.victim.join("precious.txt").exists(),
        "the decoy directory outside the state root must survive untouched"
    );
}

#[test]
fn down_rejects_a_traversal_name() {
    let home = FakeHome::new("down");

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .env("HOME", &home.root)
        .args(["down", "../victim"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not a valid sandbox name"),
        "stderr should name the actual problem, got: {stderr}"
    );
}

/// The second, independent entry point the same bug reached through:
/// an SSH `ProxyCommand` invocation's `<name>.devcroft` hostname, parsed
/// by `ssh::proxy::sandbox_name_from_host` with no validation of its own
/// — closed by the same `StatePaths::new` choke point, not by a second
/// fix in that parser.
#[test]
fn proxy_rejects_a_traversal_hostname() {
    let home = FakeHome::new("proxy");

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .env("HOME", &home.root)
        .args(["proxy", "../victim.devcroft"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert!(
        home.victim.join("precious.txt").exists(),
        "the decoy directory outside the state root must survive untouched"
    );
}

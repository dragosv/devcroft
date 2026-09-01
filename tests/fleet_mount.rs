//! `add-mount-isolation`'s namespace primitive, at the level task group 1
//! promises: entering a private mount namespace, and proving propagation
//! is actually private rather than merely that the calls returned `Ok`.
//!
//! Deliberately does not test the mount *view* (task group 2) — no
//! project root, no `/nix/store`, no policy compilation. That is a
//! separate, later piece; this file covers only what `src/fleet/mount.rs`
//! itself does, the same split `tests/fleet_netns.rs` drew between the
//! namespace primitive and the rest of fleet.

use std::io::Read;
use std::process::Command;

/// Skips rather than fails where unprivileged mount namespaces are
/// unavailable, matching `tests/fleet_netns.rs`'s own
/// `namespaces_available` — a container runtime's seccomp profile, an
/// AppArmor policy restricting unprivileged user namespaces, or an
/// exhausted `max_user_namespaces` can each deny this independently, and
/// none of them is a devcroft bug.
///
/// Deliberately asks strictly less than the tests below assert, for the
/// identical reason that function documents: `__mount_probe` only enters
/// the namespace and makes propagation private; it does not mount
/// anything or check that a mount stays invisible to the host. A gate
/// must never depend on the behaviour it gates.
fn mount_namespaces_available() -> bool {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_probe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The property [`crate::fleet::mount::make_propagation_private`] exists
/// for: a mount made after it must not appear in the host's own mount
/// namespace.
///
/// Mirrors `tests/fleet_netns.rs`'s `an_agents_service_is_not_reachable_
/// from_the_host` in shape — hold the child open, signal readiness rather
/// than sleeping a guessed interval, then check from the host's side
/// while it is genuinely still running.
#[test]
fn a_mount_made_after_entering_the_namespace_does_not_leak_to_the_host() {
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "devcroft-mount-isolation-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("marker");

    let mut child = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_isolation_sim")
        .arg(&dir)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut ready = [0u8; 5];
    let read = child
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut ready)
        .is_ok();

    // Checked from *this* process — the host's own mount namespace —
    // while the child (in its own, private-propagation namespace) is
    // still holding its tmpfs mount open over the same path.
    let leaked = marker.exists();

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(read, "the child should have signalled readiness");
    assert!(
        !leaked,
        "a mount made inside a private-propagation mount namespace must not \
         be visible from the host's own namespace, but {} existed",
        marker.display()
    );
}

/// Not a test — a compile-time reminder of what this file does *not*
/// cover, so a later reader does not mistake its scope. Matching
/// `tests/fleet_netns.rs`'s own `scope_note`.
///
/// Absent here, and correctly so: the mount *plan* (project root,
/// `/nix/store`, system layer, `/tmp`), policy compilation and
/// `policy --render`, `doctor`'s report (covered by the existing
/// `netns` probe reuse, not a new probe here), and PID namespaces
/// (left to fleet's D2, design.md Open Question 2). This file covers
/// exactly the namespace primitive task group 1 built.
#[allow(dead_code)]
fn scope_note() {}

//! **Landlock does not mediate `connect()` to a pathname unix socket —
//! and that no longer matters, because `add-mount-isolation` closes the
//! reachability gap at a different layer.**
//!
//! This file used to assert the gap itself: Landlock's network rules
//! (ABI V4+) cover TCP bind/connect for AF_INET/AF_INET6 only, so a
//! `connect()` to a pathname unix socket fell through to ordinary DAC —
//! reachable if the filesystem permissions allowed it, regardless of the
//! compiled policy, including sockets in directories the policy did not
//! grant. That was real, and closing it needed a mechanism Landlock
//! itself has no ABI for.
//!
//! **`add-mount-isolation` is that mechanism.** Every sandbox now gets
//! its own mount namespace and filesystem view
//! (`fleet::mount::construct_view`), and a socket outside that view does
//! not resolve at all — `connect()` fails with `ENOENT`, not a
//! permission error, because there is nothing left to name. Landlock
//! still governs access to what the view contains; the view now governs
//! what exists to be reached in the first place. Both tests below are
//! inverted from what this file asserted before: `__uds_probe` (`src/
//! bin/devcroft.rs`) now constructs the mount view before applying
//! Landlock, matching the real ordering `up.rs`'s `pre_exec` uses, so a
//! refusal here is the same refusal a real sandboxed session gets.
//!
//! Two consequences, both recorded in `docs/known-gaps.md`:
//!
//! - A sandbox can no longer reach a world-accessible unix socket it was
//!   not granted. The nix daemon socket is `srw-rw-rw-` by design under
//!   nix's multi-user model; a sandbox now holds none of the authority
//!   that daemon would otherwise extend to an unprivileged client —
//!   exactly the package-manager authority `sandbox-provisioning`
//!   P2a/P2b says agents must not have, now a kernel-enforced boundary
//!   rather than devcroft's own refusal to grant.
//! - The same property is still load-bearing in the *wanted* direction:
//!   a pathname unix socket crosses a network namespace, which is what
//!   lets an isolated sandbox reach devcroft's own egress proxy without
//!   a TUN device or a forwarding helper. That is why the mount view
//!   names the proxy socket back in explicitly (design.md M3) rather
//!   than only ever removing paths — asserted separately in
//!   `tests/mount_view_e2e.rs`, not here.
//!
//! **Linux-only in practice.** A mount namespace is what closes this gap,
//! and macOS has no equivalent primitive — Seatbelt alone leaves the
//! identical ungranted socket reachable (measured, macOS 15; see
//! `docs/known-gaps.md`, `docs/threat-model.md`). Both tests below gate on
//! `mount_namespaces_available()`, which reports unavailable on macOS, so
//! they self-skip there rather than asserting a refusal this host cannot
//! deliver. Getting the macOS measurement right needed one correction
//! first — Seatbelt evaluates the path as written, and `/tmp` is a symlink
//! to `/private/tmp`, so probing `/tmp/<dir>/p.sock` is refused with
//! `Operation not permitted` while the *same socket* at
//! `/private/tmp/<dir>/p.sock` connects. That is symlink traversal being
//! denied, not AF_UNIX being mediated, and reading it as the latter would
//! have published "macOS closes this gap" off a test artifact. Hence
//! `short_socket_dir` canonicalizes.
//!
//! **The abstract-socket half of the original gap is unrelated and still
//! open** (`docs/known-gaps.md`, `docs/threat-model.md`): an `@`-prefixed
//! socket has no filesystem path for a mount view to remove, so it needs
//! Landlock's own `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET`, which devcroft
//! does not yet set. Nothing here measures that half.

use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;

/// `sun_path` is 108 bytes, so this cannot live under a long scratch
/// path. Found the hard way while spiking this: the first attempt used a
/// deep temp directory and failed with "path must be shorter than
/// SUN_LEN" in *both* the granted and ungranted runs, which looks exactly
/// like "refused" and says nothing at all. devcroft already guards this
/// limit for service supervisor sockets (`UpError::Config`).
///
/// Creates the directory and returns its **canonical** path, for the
/// second reason the module doc gives: on macOS `/tmp` is a symlink, and
/// probing through it is denied for the symlink rather than for the
/// socket. `/private/tmp/dcuds<pid>` is still far inside SUN_LEN.
fn short_socket_dir() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(format!("/tmp/dcuds{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

/// Skips rather than fails where unprivileged mount namespaces are
/// unavailable — matching every other namespace-gated test in this
/// project (`tests/fleet_netns.rs`, `tests/fleet_mount.rs`): a container
/// runtime's seccomp profile, an AppArmor policy restricting
/// unprivileged user namespaces, or an exhausted `max_user_namespaces`
/// can each deny this independently, and none is a devcroft bug. Without
/// it, `__uds_probe` cannot construct a view at all, and connecting
/// would prove nothing about the property under test.
fn mount_namespaces_available() -> bool {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_probe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn a_sandboxed_process_cannot_reach_an_ungranted_unix_socket() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }

    let dir = short_socket_dir();
    let sock = dir.join("p.sock");
    // Held open for the probe to attempt against, but **not** accepted
    // from — unlike this file's pre-inversion version, which spawned a
    // thread blocked on `listener.accept()`. That thread assumed the
    // probe would connect, which was true of the gap this file used to
    // assert; now that the connect is expected to fail, an `accept()`
    // that never receives one hangs forever. The listener merely needs
    // to *exist* for the probe's `connect()` to have a real target to be
    // refused from — a socket nothing is listening to would prove
    // nothing (the connect could fail for either reason).
    let _listener = UnixListener::bind(&sock).unwrap();

    // The probe grants only its own cwd, then constructs a real mount
    // view before applying Landlock (`__uds_probe`'s own doc). The
    // socket is outside both, under /tmp — this is the case a mount
    // view removes.
    let cwd = std::env::temp_dir();
    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__uds_probe")
        .arg(&sock)
        .current_dir(&cwd)
        .output()
        .unwrap();

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !out.status.success(),
        "expected the sandboxed probe to be refused reaching an ungranted unix \
         socket — add-mount-isolation's own view should have made the path not \
         resolve. If this now SUCCEEDS, the fix has regressed. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("No such file or directory"),
        "expected refusal because the path does not resolve (spec: \"the failure \
         is that the path does not resolve, not that a rule refused it\"), got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The concrete instance that matters, asserted separately from the
/// general property: this is the one that contradicted a devcroft
/// invariant rather than being an abstract limitation.
#[test]
fn a_sandboxed_process_cannot_reach_the_nix_daemon_socket_if_one_exists() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !mount_namespaces_available() {
        eprintln!("skipping: this host cannot create unprivileged mount namespaces");
        return;
    }
    let nix_sock = Path::new("/nix/var/nix/daemon-socket/socket");
    if !nix_sock.exists() {
        eprintln!("skipping: no nix daemon socket on this host");
        return;
    }
    // The control, and it is not optional. The assertion below concludes
    // "the gap is closed" from a failed connect, and a socket file whose
    // daemon is not running produces exactly that failure for an
    // unrelated reason — so without this, a host with a dead `nix-daemon`
    // would report a closed gap that was never actually exercised.
    // Connecting from *outside* any sandbox first turns the assertion
    // into the implication it is meant to be — reachable out here, so it
    // must not be reachable in there — rather than a bare claim about
    // one connect() call.
    if std::os::unix::net::UnixStream::connect(nix_sock).is_err() {
        eprintln!(
            "skipping: the nix daemon socket exists but nothing is listening, so an \
             unreachable socket inside the sandbox would prove nothing"
        );
        return;
    }

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__uds_probe")
        .arg(nix_sock)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "expected a sandboxed process to be refused reaching the nix daemon socket \
         with /nix/var absent from its mount view — the gap docs/known-gaps.md \
         documents as closed. If this now SUCCEEDS, the fix has regressed. \
         stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

//! **A sandbox cannot reach a pathname unix socket it was not granted —
//! on both platforms, through two genuinely different mechanisms.**
//!
//! This file used to assert the *gap*: neither backend mediated
//! `connect()` to a pathname unix socket, so it fell through to ordinary
//! filesystem permissions and was reachable from inside a sandbox whose
//! compiled policy granted none of it. The nix daemon socket
//! (`srw-rw-rw-` by design under nix's multi-user model) was the instance
//! that mattered, because reaching it hands a sandbox exactly the
//! package-manager authority `sandbox-provisioning` P2a/P2b says agents
//! must not have.
//!
//! Both halves are now closed, and **the two platforms close them on
//! different axes** — which is why the assertions below are split rather
//! than shared:
//!
//! - **Linux — the filesystem axis** (`add-mount-isolation`). Landlock's
//!   network rules cover TCP for AF_INET/AF_INET6 only; no Landlock ABI
//!   expresses AF_UNIX at all, so the fix could not be a rule. Every
//!   sandbox instead gets its own mount namespace and filesystem view
//!   (`fleet::mount::construct_view`), and a socket outside that view
//!   does not resolve: `connect()` fails **`ENOENT`**, because there is
//!   nothing left to name.
//!
//! - **macOS — the network axis** (`add-macos-unix-socket-scoping`).
//!   Seatbelt has no mount-namespace equivalent, and needs none: it
//!   classifies a unix-socket `connect()` as `network-outbound`, not as
//!   filesystem access. So the deny-default network mode devcroft already
//!   compiles for `network.default = "deny"` is what mediates AF_UNIX,
//!   and an ungranted socket fails **`EPERM`** against a path that still
//!   resolves perfectly well.
//!
//! Forcing one assertion to cover both would be vacuously true on
//! whichever platform it was not written for (`add-macos-unix-socket-
//! scoping` design.md S3), so each platform asserts its own failure
//! shape, and each asserts a *positive* control alongside it — a granted
//! socket that does connect. Without that control an `EPERM` proves only
//! that something went wrong, not that the policy is what refused.
//!
//! **Measured, not inferred.** The macOS half was confirmed live on macOS
//! 15.7.4 (arm64) against this host's real `nix-daemon` socket before any
//! of it was claimed, per this capability's own "verified before claimed"
//! requirement. Three results from that spike are load-bearing here:
//!
//! - With the network unrestricted, the sandbox connects to the nix
//!   daemon socket **even though `stat()` on the same path is denied** —
//!   `connect()` is not gated by the filesystem layer in any way. That is
//!   the gap, reproduced live.
//! - With `network.default = "deny"`, the same connect is refused
//!   `EPERM`, with no devcroft code change required: the mechanism was
//!   already shipping, just never measured or claimed.
//! - A `filesystem.allow` grant for the socket's own path does **not**
//!   admit it (`stat()` starts succeeding; `connect()` stays refused).
//!   Filesystem and unix-socket grants are orthogonal layers in the
//!   backend library, so on macOS only an explicit unix-socket grant
//!   opens one — see `docs/known-gaps.md` for why that asymmetry with
//!   Linux is a published limitation rather than a bug.
//!
//! **Two things this file does not cover.** *Abstract* sockets are a
//! separate half, Linux-only as a concept, measured in
//! `tests/abstract_socket_not_reachable.rs`. And on macOS the guarantee
//! is scoped to deny-default sandboxes: an `allow`-default macOS sandbox
//! still reaches any world-accessible socket, where a Linux one does not,
//! because a mount view removes the path regardless of network mode. Both
//! are recorded in `docs/known-gaps.md`.

use std::path::Path;
use std::process::Command;

/// Creates this test's scratch directory and returns it **canonicalized**.
///
/// Two separate traps are folded into this one helper, both of which
/// produce a refusal for the wrong reason rather than a failure:
///
/// - `sun_path` is 108 bytes, so this cannot live under a long scratch
///   path. Found the hard way while spiking this originally: a deep temp
///   directory failed with "path must be shorter than SUN_LEN" in *both*
///   the granted and ungranted runs, which looked exactly like "refused"
///   and said nothing at all. devcroft already guards this limit for
///   service supervisor sockets (`UpError::Config`). Hence `/tmp`.
///
/// - **The canonicalization, which matters only on macOS and matters a
///   lot.** `/tmp` there is a symlink to `/private/tmp`, and resolving it
///   is a filesystem read the sandboxed probe has not been granted — so
///   `connect("/tmp/…")` fails `EPERM` during path resolution no matter
///   what the network policy says. That is indistinguishable from the
///   refusal these tests are trying to observe, and it made two of them
///   pass with the deny rule deliberately removed. Handing the probe the
///   already-resolved `/private/tmp/…` form removes the symlink hop, so
///   the network rule is the only thing left that can refuse.
fn make_socket_dir(suffix: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(format!("/tmp/dcuds{}{}", std::process::id(), suffix));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// The socket the ungranted/granted runs both aim at, held open for the
/// probe to attempt against but **not** accepted from.
///
/// Deliberately not `accept()`ing: an earlier version of this file
/// spawned a thread blocked on `accept()`, which assumed the probe would
/// connect — true of the gap this file used to assert, and a hang now
/// that the connect is expected to fail. The listener merely needs to
/// *exist*, so that a refusal is attributable to the policy rather than
/// to there being nothing at the other end.
fn bind_listener(sock: &Path) -> std::os::unix::net::UnixListener {
    std::os::unix::net::UnixListener::bind(sock).unwrap()
}

/// Runs `__uds_probe` against `sock` from `cwd`, which is the only path
/// the probe grants itself.
///
/// **`cwd` is a parameter, and which directory each platform passes is
/// load-bearing** — see the two call-site helpers below. Getting it wrong
/// does not fail the test; it makes the test pass for the wrong reason,
/// which is exactly what happened while this file was being written.
fn probe_from(sock: &Path, grant: bool, cwd: &Path) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_devcroft"));
    cmd.arg("__uds_probe").arg(sock);
    if grant {
        cmd.arg("--grant");
    }
    cmd.current_dir(cwd).output().unwrap()
}

/// Linux: run from a directory that is *not* the socket's own, so the
/// socket falls outside the mount view the probe builds for itself. That
/// exclusion is the mechanism under test.
#[cfg(target_os = "linux")]
fn probe(sock: &Path, grant: bool) -> std::process::Output {
    probe_from(sock, grant, &std::env::temp_dir())
}

/// macOS: run from the socket's **own directory**, so the socket is
/// filesystem-granted, and a refusal can only have come from the network
/// deny rule.
///
/// **This is the opposite of the Linux choice, deliberately, and it was
/// found by a teeth check rather than by reasoning.** The obvious
/// arrangement — cwd elsewhere, socket ungranted, mirroring Linux — makes
/// these tests *vacuous* on macOS: the sockets live under `/tmp`, which is
/// a symlink to `/private/tmp`, and resolving that symlink needs a
/// filesystem read the probe has not granted. `connect()` then fails
/// `EPERM` during path resolution no matter what the network policy says,
/// so the assertions passed with the deny rule deliberately removed.
///
/// Granting the socket's directory removes that confound without
/// weakening anything, because the two layers are orthogonal (measured:
/// a filesystem grant for a socket makes `stat()` succeed and leaves
/// `connect()` refused — nono #696). So the socket here is fully
/// reachable as a *file* and still refused as a *socket*, which is a
/// sharper statement of the property than the ungranted arrangement could
/// make.
#[cfg(target_os = "macos")]
fn probe(sock: &Path, grant: bool) -> std::process::Output {
    probe_from(sock, grant, sock.parent().unwrap())
}

/// The nix daemon socket, if this host has a live one.
///
/// Returns `None` — and the caller skips — unless a connect from
/// *outside* any sandbox succeeds first. That control is not optional:
/// the assertions conclude "the gap is closed" from a failed connect, and
/// a socket file whose daemon is not running produces exactly that
/// failure for an unrelated reason. Connecting out here first turns the
/// assertion into the implication it is meant to be — reachable out
/// there, so it must not be reachable in here.
fn live_nix_daemon_socket() -> Option<&'static Path> {
    let sock = Path::new("/nix/var/nix/daemon-socket/socket");
    if !sock.exists() {
        eprintln!("skipping: no nix daemon socket on this host");
        return None;
    }
    if std::os::unix::net::UnixStream::connect(sock).is_err() {
        eprintln!(
            "skipping: the nix daemon socket exists but nothing is listening, so an \
             unreachable socket inside the sandbox would prove nothing"
        );
        return None;
    }
    Some(sock)
}

// ---------------------------------------------------------------------
// Linux: the filesystem axis. An ungranted socket's path does not exist.
// ---------------------------------------------------------------------

/// Skips rather than fails where unprivileged mount namespaces are
/// unavailable — matching every other namespace-gated test in this
/// project (`tests/fleet_netns.rs`, `tests/fleet_mount.rs`): a container
/// runtime's seccomp profile, an AppArmor policy restricting
/// unprivileged user namespaces, or an exhausted `max_user_namespaces`
/// can each deny this independently, and none is a devcroft bug. Without
/// it, `__uds_probe` cannot construct a view at all, and connecting
/// would prove nothing about the property under test.
#[cfg(target_os = "linux")]
fn mount_namespaces_available() -> bool {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__mount_probe")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
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

    let dir = make_socket_dir("");
    let sock = dir.join("p.sock");
    let _listener = bind_listener(&sock);

    let out = probe(&sock, false);
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
#[cfg(target_os = "linux")]
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
    let Some(nix_sock) = live_nix_daemon_socket() else {
        return;
    };

    let out = probe(nix_sock, false);
    assert!(
        !out.status.success(),
        "expected a sandboxed process to be refused reaching the nix daemon socket \
         with /nix/var absent from its mount view — the gap docs/known-gaps.md \
         documents as closed. If this now SUCCEEDS, the fix has regressed. \
         stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------
// macOS: the network axis. The path resolves; the deny rule refuses it.
// ---------------------------------------------------------------------

/// `EPERM`, as Seatbelt reports a denied `network-outbound`.
///
/// Matched on the message rather than a raw errno because the probe
/// reports through `io::Error`'s `Display`; the distinction that carries
/// the meaning is against Linux's `ENOENT` ("No such file or directory"),
/// and these two strings cannot be confused for one another.
#[cfg(target_os = "macos")]
const SEATBELT_REFUSAL: &str = "Operation not permitted";

#[cfg(target_os = "macos")]
#[test]
fn a_sandboxed_process_cannot_reach_an_ungranted_unix_socket() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    let dir = make_socket_dir("");
    let sock = dir.join("p.sock");
    let _listener = bind_listener(&sock);

    let ungranted = probe(&sock, false);
    // The positive control, and it is what makes the assertion above it
    // mean anything: same probe, same socket, same everything except an
    // explicit unix-socket grant. If this run also failed, the refusal
    // would be measuring some setup problem rather than the deny rule.
    let granted = probe(&sock, true);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !ungranted.status.success(),
        "expected the sandboxed probe to be refused reaching an ungranted unix \
         socket — Seatbelt's `(deny network*)` covers unix-socket connect(). \
         If this now SUCCEEDS, the gap has reopened. stderr: {}",
        String::from_utf8_lossy(&ungranted.stderr)
    );
    assert!(
        String::from_utf8_lossy(&ungranted.stderr).contains(SEATBELT_REFUSAL),
        "expected refusal by the network deny rule (EPERM) rather than by the \
         path failing to resolve — macOS has no mount view, so the path is still \
         perfectly nameable and it is the rule that refuses. got: {}",
        String::from_utf8_lossy(&ungranted.stderr)
    );
    assert!(
        granted.status.success(),
        "expected an explicitly granted unix socket to remain reachable — this is \
         the control that proves the refusal above comes from the policy and not \
         from a broken probe, and it is the same mechanism that keeps a sandbox's \
         own egress path open. stderr: {}",
        String::from_utf8_lossy(&granted.stderr)
    );
}

/// A grant admits the socket it names and no other — the scoping half of
/// this capability's second requirement ("the scoped grant admits its own
/// proxy socket and no other sandbox's"), asserted on plain sockets
/// because the property is about the grant, not about the proxy.
#[cfg(target_os = "macos")]
#[test]
fn a_unix_socket_grant_admits_only_the_socket_it_names() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    let dir = make_socket_dir("scope");
    let mine = dir.join("mine.sock");
    let theirs = dir.join("theirs.sock");
    let _mine = bind_listener(&mine);
    let _theirs = bind_listener(&theirs);

    // Granted its own socket; the *other* one shares a directory with it
    // and is otherwise identical, so anything that admitted both would be
    // granting by directory rather than by path.
    let own = probe(&mine, true);
    let other = probe(&theirs, false);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        own.status.success(),
        "expected the granted socket to be reachable. stderr: {}",
        String::from_utf8_lossy(&own.stderr)
    );
    assert!(
        !other.status.success(),
        "expected a socket in the same directory as a granted one to stay refused \
         — a grant is scoped to the path it names, not to its parent. If this now \
         SUCCEEDS, the grant has widened. stderr: {}",
        String::from_utf8_lossy(&other.stderr)
    );
}

/// The concrete instance that matters, same reasoning as the Linux twin
/// above: reaching this socket is the package-manager authority
/// `sandbox-provisioning` P2a/P2b says an agent must not hold.
#[cfg(target_os = "macos")]
#[test]
fn a_sandboxed_process_cannot_reach_the_nix_daemon_socket_if_one_exists() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let Some(nix_sock) = live_nix_daemon_socket() else {
        return;
    };

    let out = probe(nix_sock, false);
    assert!(
        !out.status.success(),
        "expected a sandboxed process to be refused reaching the nix daemon socket \
         under `network.default = \"deny\"` — the gap docs/known-gaps.md documents \
         as closed on macOS. If this now SUCCEEDS, the fix has regressed. \
         stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(SEATBELT_REFUSAL),
        "expected the network deny rule to be what refused it. got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

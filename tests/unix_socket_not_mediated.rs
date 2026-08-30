//! **Landlock does not mediate `connect()` to a pathname unix socket.**
//!
//! This test asserts a *gap*, not a guarantee — it documents behaviour
//! devcroft currently cannot prevent, so the gap is measurable rather
//! than folklore, and so anything that closes it makes a test fail loudly
//! instead of leaving a stale claim in the docs.
//!
//! Landlock's network rules (ABI V4+) cover TCP bind/connect for
//! AF_INET/AF_INET6 only. AF_UNIX connect falls through to ordinary DAC:
//! if the filesystem permissions on the socket allow it, a sandboxed
//! process reaches it regardless of the compiled policy — including
//! sockets in directories the policy explicitly does not grant.
//!
//! Two consequences, both recorded in `docs/known-gaps.md`:
//!
//! - A sandbox can reach any world-accessible unix socket on the host.
//!   The nix daemon socket is `srw-rw-rw-` by design under nix's
//!   multi-user model, so a sandbox holds whatever authority that daemon
//!   grants an unprivileged client — which is the package-manager
//!   authority `sandbox-provisioning` P2a/P2b says agents must not have.
//! - The same property is what makes an *isolated* sandbox able to reach
//!   devcroft's own egress proxy: a unix socket crosses a network
//!   namespace, so no TUN device or forwarding helper is needed. One
//!   mechanism, one useful consequence and one unwanted one.
//!
//! Closing it needs a mount namespace — `add-mount-isolation`, whose task
//! 4.1 is to invert both tests below. Measured: masking a path inside an
//! unprivileged `unshare(CLONE_NEWUSER | CLONE_NEWNS)` turns the connect
//! into `No such file or directory`. Seccomp filtering on `connect()`
//! would also work and was this file's original answer, but it filters a
//! syscall whose argument still names a real path; removing the path
//! closes the whole class.

use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;

/// `sun_path` is 108 bytes, so this cannot live under a long scratch
/// path. Found the hard way while spiking this: the first attempt used a
/// deep temp directory and failed with "path must be shorter than
/// SUN_LEN" in *both* the granted and ungranted runs, which looks exactly
/// like "Landlock denied it" and says nothing at all. devcroft already
/// guards this limit for service supervisor sockets (`UpError::Config`).
fn short_socket_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/tmp/dcuds{}", std::process::id()))
}

#[test]
fn a_sandboxed_process_reaches_an_ungranted_unix_socket() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    let dir = short_socket_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("p.sock");
    let listener = UnixListener::bind(&sock).unwrap();

    let accepted = std::thread::spawn(move || {
        let mut buf = [0u8; 8];
        match listener.accept() {
            Ok((mut c, _)) => c.read(&mut buf).is_ok(),
            Err(_) => false,
        }
    });

    // The probe grants only its own cwd. The socket is outside it, under
    // /tmp, which the compiled policy does not grant — so a Landlock
    // mediating AF_UNIX would refuse this.
    let cwd = std::env::temp_dir();
    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__uds_probe")
        .arg(&sock)
        .current_dir(&cwd)
        .output()
        .unwrap();

    let connected = out.status.success();
    let _ = accepted.join();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        connected,
        "expected the sandboxed probe to reach an ungranted unix socket, since \
         Landlock's network rules cover TCP only. If this now FAILS, the gap has \
         closed — either the kernel gained AF_UNIX mediation or devcroft added \
         seccomp filtering. That is good news, and `docs/known-gaps.md`, \
         `docs/threat-model.md` and `sandbox-provisioning`'s design.md all claim \
         it is open and must be corrected together. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The concrete instance that matters, asserted separately from the
/// general property: this is the one that contradicts a devcroft
/// invariant rather than being an abstract limitation.
#[test]
fn a_sandboxed_process_reaches_the_nix_daemon_socket_if_one_exists() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let nix_sock = Path::new("/nix/var/nix/daemon-socket/socket");
    if !nix_sock.exists() {
        eprintln!("skipping: no nix daemon socket on this host");
        return;
    }

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("__uds_probe")
        .arg(nix_sock)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected a sandboxed process to reach the nix daemon socket with /nix \
         ungranted — the gap `docs/known-gaps.md` documents. If this now FAILS, \
         the gap has closed and those docs need correcting. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

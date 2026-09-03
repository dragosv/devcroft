//! `add-backend-capabilities` task 1.5: does devcroft's own, unmodified
//! `CapabilitySet` already deny `connect()` to an *abstract* unix socket
//! (`@`-prefixed, no filesystem path)?
//!
//! This is the other half of the AF_UNIX gap `tests/
//! unix_socket_not_mediated.rs` closes the pathname half of. A mount
//! view has nothing to remove here — an abstract socket has no path —
//! so the only lever is Landlock's own `Scope::AbstractUnixSocket`
//! (ABI V6+), requested whenever `nono`'s `CapabilitySet::ipc_mode()` is
//! `IpcMode::SharedMemoryOnly`. That is `nono`'s own `#[default]`, and
//! `policy::capability_set.rs` never calls `set_ipc_mode` at all — so
//! this was already true on every devcroft sandbox before this file, or
//! `add-backend-capabilities`, existed. This test is what turns that
//! trace-through-the-library reading into a live measurement, matching
//! how `unix_socket_not_mediated.rs` already treats the pathname half:
//! evidence, not inference (design.md C3).

// Abstract unix sockets are a Linux-only address family: there is no
// `SocketAddrExt` to import, and no gap to measure, on any other
// platform. The whole file is gated rather than each test, and a
// non-Linux stub below skips out loud — same convention every other e2e
// test here follows, so a skip is visible in `--nocapture` instead of
// the file silently disappearing from the run.
#[cfg(target_os = "linux")]
mod linux {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};
    use std::process::Command;

    /// A fresh name per test run, so a leftover listener from a killed prior
    /// run (abstract sockets have no filesystem entry to clean up — they
    /// vanish only when every holder of the fd closes it) can never collide
    /// with this one.
    fn abstract_name() -> String {
        format!("devcroft-abstract-test-{}", std::process::id())
    }

    /// Whether this kernel's Landlock ABI supports scoping at all — the
    /// precondition for `Scope::AbstractUnixSocket` to be requestable in the
    /// first place. Distinct from `backend_supported()` (any Landlock at
    /// all): a V4/V5 kernel has *a* working sandbox and would still fail
    /// this test for a reason that is the kernel's, not devcroft's.
    /// `nono::Sandbox::support_info` is the same real ABI probe
    /// `policy::degraded::backend_support` already uses elsewhere — its
    /// `details` string names every feature the detected ABI has, "Signal
    /// and abstract UNIX socket scoping" among them when V6 is present.
    fn landlock_scoping_available() -> bool {
        devcroft::policy::backend_support()
            .details
            .contains("abstract UNIX socket scoping")
    }

    #[test]
    fn devcrofts_default_capability_set_refuses_an_abstract_socket() {
        if !devcroft::policy::backend_supported() {
            eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
            return;
        }
        if !landlock_scoping_available() {
            eprintln!(
                "skipping: this kernel's Landlock ABI has no scoping support \
             (needs V6+); the abstract-socket gap is real here, not closed"
            );
            return;
        }

        let name = abstract_name();
        let addr = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        // Held open so the probe has a real target to be refused from — a
        // socket nothing is listening on would prove nothing (the connect
        // could fail for either reason), same reasoning `tests/
        // unix_socket_not_mediated.rs` already applies to its own listener.
        let _listener = UnixListener::bind_addr(&addr).unwrap();

        let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
            .arg("__abstract_socket_probe")
            .arg(&name)
            .current_dir(std::env::temp_dir())
            .output()
            .unwrap();

        assert!(
            !out.status.success(),
            "expected devcroft's own default CapabilitySet to refuse an abstract \
         unix socket via Landlock's Scope::AbstractUnixSocket (IpcMode:: \
         SharedMemoryOnly is nono's own default, never overridden). If this \
         now SUCCEEDS, either nono's default changed or something started \
         calling set_ipc_mode(Full) — either way the matrix entry in \
         add-backend-capabilities needs correcting, not just this test. \
         stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The control this file's main assertion depends on: an *unrestricted*
    /// process reaching the same abstract socket, so a future failure of the
    /// test above is known to mean "Landlock refused it", not "nothing was
    /// listening" or "abstract sockets don't work like this on this kernel".
    #[test]
    fn an_unrestricted_process_can_reach_the_same_abstract_socket() {
        let name = format!("{}-control", abstract_name());
        let addr = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let listener = UnixListener::bind_addr(&addr).unwrap();
        let accepted = std::thread::spawn(move || listener.accept().is_ok());

        let connected = std::os::unix::net::UnixStream::connect_addr(&addr).is_ok();
        let _ = accepted.join();

        assert!(
            connected,
            "an unrestricted process must be able to reach an abstract socket — \
         if this fails, abstract sockets don't work the way this test file \
         assumes on this host, which would make the main test's refusal \
         meaningless rather than a measurement"
        );
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn devcrofts_default_capability_set_refuses_an_abstract_socket() {
    eprintln!(
        "skipping: abstract unix sockets are Linux-only; this platform has no \
         equivalent address family to measure"
    );
}

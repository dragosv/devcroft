//! **UDP must not escape a deny-network sandbox.**
//!
//! Written after finding it did. A sandbox with
//! `network.default = "deny"` and an allowlist naming one host completed a
//! full DNS round-trip to `8.8.8.8:53` — socket created, query sent, 61
//! bytes of reply received. The allowlist constrained nothing, because it
//! was never in the path.
//!
//! **Root cause, and it is a shape this project has hit before.**
//! Landlock's network rules are TCP-only: `NetPort` gates `connect`/`bind`
//! for AF_INET *stream* sockets and says nothing about datagrams. nono
//! does ship a seccomp filter that denies UDP, raw and non-stream
//! sockets — but it is `apply_auto`'s fallback for pre-V4 Landlock
//! kernels, and on a V6 host it is never installed. `add-egress-proxy`
//! task 0 found the identical trap with `install_seccomp_proxy_filter`:
//! the library has two enforcement paths, and the modern one does not
//! cover everything the fallback did. Reading nono's source for "does it
//! deny UDP" returns yes, and that answer is about the wrong path.
//!
//! **The fix is a network namespace, not a filter.** An isolated sandbox
//! has no route out at all, so UDP fails with `ENETUNREACH` regardless of
//! which protocols the policy layer covers.
//! `CompiledPolicy::wants_network_isolation` therefore returns true for
//! *every* `network_block` sandbox, not only those declaring ports —
//! that second condition was removed for this reason.
//!
//! Egress that is wanted still works: it goes through the keeper's relay
//! to the host proxy, which is TCP to a local port
//! (`tests/isolated_egress_e2e.rs` asserts that, and would fail if this
//! fix had closed egress along with the leak).
//!
//! **Neutral surface** (`test-runtime-fixture`): the namespace is the
//! backend's, whatever the provider. It used to `flox init` + `flox install
//! python3`; it now runs on any row with `network_probes`.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::process::Command;

/// A real DNS query to a public resolver, not merely a `sendto`. A send
/// can succeed into a void; a *reply* proves bidirectional egress reached
/// the internet and came back, which is the property that matters.
const UDP_PROBE: &str = "\
import socket\n\
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)\n\
s.settimeout(3)\n\
try:\n\
\x20   q = b'\\x00\\x00\\x01\\x00\\x00\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x07example\\x03com\\x00\\x00\\x01\\x00\\x01'\n\
\x20   s.sendto(q, ('8.8.8.8', 53))\n\
\x20   d, _ = s.recvfrom(512)\n\
\x20   print('LEAK', len(d))\n\
except Exception as e:\n\
\x20   print('DENIED', type(e).__name__)\n";

/// The control: the same query from the host, outside any sandbox, gets
/// a reply. Without it, a host with no route to 8.8.8.8 (an offline CI
/// runner, a firewalled network) would print `DENIED` from inside the
/// sandbox too, and the test would pass having measured the network rather
/// than the boundary.
fn host_gets_a_dns_reply() -> bool {
    let q =
        b"\x00\x00\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01";
    let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") else {
        return false;
    };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let mut buf = [0u8; 512];
    s.send_to(q, "8.8.8.8:53").is_ok() && s.recv_from(&mut buf).is_ok()
}

#[test]
fn a_deny_network_sandbox_cannot_send_udp() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !devcroft::fleet::netns::probe(std::path::Path::new(env!("CARGO_BIN_EXE_devcroft")))
        .unwrap_or(false)
    {
        // Deliberately a skip and not a pass: without namespaces this
        // host *does* leak, and the honest report is "not tested here",
        // not "fine". `up` warns in that case.
        eprintln!("skipping: no unprivileged network namespaces; this host cannot enforce it");
        return;
    }
    if !host_gets_a_dns_reply() {
        eprintln!(
            "skipping: the host itself gets no DNS reply from 8.8.8.8, so a denial inside would mean nothing"
        );
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    for_each_row("udp", |fx| {
        if !fx.capabilities().network_probes {
            eprintln!("row {}: skipped (no python3 in the sandbox)", fx.name());
            return;
        }
        // An allowlist naming one TCP host, and no declared ports. This is
        // the exact shape that leaked: it does not qualify for isolation
        // under the old "must declare ports or services" condition, and its
        // allowlist has nothing to say about datagrams.
        let path = fx.project_root().join("devcroft.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            format!("{text}\n[network]\ndefault = \"deny\"\nallow = [\"example.com\"]\n"),
        )
        .unwrap();
        let sandbox_name = fx.sandbox_name();
        let paths = StatePaths::new(sandbox_name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
        let _ = std::fs::remove_file(&paths.lifecycle_lock);

        assert_eq!(
            fx.bring_up(&UpOptions::default())
                .unwrap_or_else(|e| panic!("row {}: up failed: {e}", fx.name())),
            UpOutcome::Started
        );

        let out = Command::new(devcroft_bin)
            .arg("exec")
            .arg(sandbox_name)
            .arg("--")
            .arg("python3")
            .arg("-c")
            .arg(UDP_PROBE)
            .output()
            .unwrap();
        let result = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        down(sandbox_name).unwrap();

        assert!(
            result.contains("DENIED"),
            "a `network.default = \"deny\"` sandbox completed a UDP round-trip to a public \
         DNS resolver. Landlock's network rules are TCP-only, so the allowlist never \
         saw this traffic — the network namespace is what denies it, and something has \
         stopped applying one to this manifest shape. Check \
         `CompiledPolicy::wants_network_isolation`: it must be true for every \
         `network_block` sandbox, not only those declaring ports. Got: {result}"
        );
    });
}

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

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
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

fn flox_project_with_python() -> Option<std::path::PathBuf> {
    if Command::new("flox").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        return None;
    }
    let root = std::env::temp_dir().join(format!("devcroft-udp-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).ok()?;
    for args in [vec!["init"], vec!["install", "python3"]] {
        if !Command::new("flox")
            .args(&args)
            .current_dir(&root)
            .output()
            .ok()?
            .status
            .success()
        {
            return None;
        }
    }
    Some(root)
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
    let Some(project_root) = flox_project_with_python() else {
        eprintln!("skipping: flox unavailable or environment setup failed");
        return;
    };

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    // An allowlist naming one TCP host, and no declared ports. This is
    // the exact shape that leaked: it does not qualify for isolation
    // under the old "must declare ports or services" condition, and its
    // allowlist has nothing to say about datagrams.
    let sandbox_name = format!("e2eudp{}", std::process::id());
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n\
         [network]\ndefault = \"deny\"\nallow = [\"example.com\"]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let out = Command::new(devcroft_bin)
        .arg("exec")
        .arg(&sandbox_name)
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

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);

    assert!(
        result.contains("DENIED"),
        "a `network.default = \"deny\"` sandbox completed a UDP round-trip to a public \
         DNS resolver. Landlock's network rules are TCP-only, so the allowlist never \
         saw this traffic — the network namespace is what denies it, and something has \
         stopped applying one to this manifest shape. Check \
         `CompiledPolicy::wants_network_isolation`: it must be true for every \
         `network_block` sandbox, not only those declaring ports. Got: {result}"
    );
}

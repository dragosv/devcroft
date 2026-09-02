//! **Both properties at once**: a sandbox with its own network namespace
//! (private port table) *and* working filtered egress.
//!
//! This is the combination a coding agent actually needs — its own
//! Postgres on the committed port, and reachable package registries or
//! model APIs — and it was the one shape devcroft could not produce. The
//! two were mutually exclusive by construction until the assumption
//! underneath was measured and found wrong.
//!
//! The assumption: "an isolated namespace has no route to the host-bound
//! proxy, so it needs a forwarding helper (pasta/slirp4netns), which
//! needs `/dev/net/tun`, which this host lacks." True of *IP routing*,
//! and irrelevant — devcroft never needed IP routing, only TCP streams
//! reaching a proxy. A **pathname unix socket crosses a network
//! namespace**, so the proxy grew a unix listener and the keeper relays
//! to it from inside the namespace. See
//! `tests/unix_socket_not_mediated.rs` for the property itself, measured
//! separately, and `docs/known-gaps.md` for its unwanted twin.
//!
//! What this asserts that the two existing tests do not:
//! `tests/network_isolation_e2e.rs` proves isolation with *no* egress;
//! `tests/egress_proxy_e2e.rs` proves egress with *no* isolation. Neither
//! would notice the combination regressing.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

/// Accepts connections and replies with a fixed 200 OK. Loops rather than
/// serving once: the allowed host is probed after the port check, so a
/// one-shot fixture would make the second request fail for the wrong
/// reason.
fn serve_ok_responses(listener: TcpListener) {
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
}

/// Same loopback-alias guard as `tests/egress_proxy_e2e.rs`: the whole of
/// `127.0.0.0/8` is bindable on Linux but only `127.0.0.1` is on macOS.
/// This test already self-skips off Linux (no unprivileged netns
/// elsewhere), so this exists so a bind failure reads as a host
/// capability rather than as a devcroft regression.
fn bind_loopback_alias(addr: &str) -> Option<TcpListener> {
    match TcpListener::bind(addr) {
        Ok(listener) => Some(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrNotAvailable => {
            let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
            eprintln!(
                "skipping: this host cannot bind the loopback alias {host} ({e}); \
                 on macOS run `sudo ifconfig lo0 alias {host} up` first"
            );
            None
        }
        Err(e) => panic!("binding {addr} failed for an unexpected reason: {e}"),
    }
}

#[test]
fn an_isolated_sandbox_still_reaches_its_allowlisted_hosts() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if !devcroft::fleet::netns::probe(std::path::Path::new(env!("CARGO_BIN_EXE_devcroft")))
        .unwrap_or(false)
    {
        eprintln!("skipping: this host cannot create unprivileged network namespaces");
        return;
    }
    if Command::new("flox").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-iso-egress-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    let init = Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        return;
    }
    let install = Command::new("flox")
        .args(["install", "curl", "python3"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        return;
    }

    // The upstreams live on the *host*. An isolated sandbox has no route
    // to them at all except through the proxy, which is exactly the
    // point: reaching the allowed one proves the relay works end to end,
    // and it cannot be an accident of shared loopback.
    let Some(allowed_listener) = bind_loopback_alias("127.0.0.3:0") else {
        return;
    };
    let allowed_port = allowed_listener.local_addr().unwrap().port();
    serve_ok_responses(allowed_listener);

    let Some(denied_listener) = bind_loopback_alias("127.0.0.4:0") else {
        return;
    };
    let denied_port = denied_listener.local_addr().unwrap().port();
    serve_ok_responses(denied_listener);

    // A service port *and* an allowlist: this manifest is what makes the
    // sandbox qualify for isolation while still wanting egress.
    const SERVICE_PORT: u16 = 18330;
    let sandbox_name = format!("e2eisoeg{}", std::process::id());
    let manifest_text = format!(
        "[sandbox]\nname = {sandbox_name:?}\n\n\
         [network]\ndefault = \"deny\"\nallow = [\"127.0.0.3\"]\nports = [{SERVICE_PORT}]\n"
    );
    let (manifest, _) = parse(&manifest_text).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // 1. The sandbox really is isolated: it binds its declared port even
    //    while the *host* holds the same number, which a shared port
    //    table would refuse with EADDRINUSE.
    let host_holder = TcpListener::bind(("127.0.0.1", SERVICE_PORT));
    let bind_probe = Command::new(devcroft_bin)
        .arg("exec")
        .arg(&sandbox_name)
        .arg("--")
        .arg("python3")
        .arg("-c")
        .arg(format!(
            "import socket\n\
             s = socket.socket()\n\
             try:\n\
             \x20   s.bind(('127.0.0.1', {SERVICE_PORT}))\n\
             \x20   s.listen(1)\n\
             \x20   print('BOUND')\n\
             except Exception as e:\n\
             \x20   print('DENIED', e)\n"
        ))
        .output()
        .unwrap();
    let bind_out = String::from_utf8_lossy(&bind_probe.stdout).to_string();

    // 2. Egress still works through the proxy, decided by hostname.
    let curl_status = |port: u16, host: &str| -> String {
        let out = Command::new(devcroft_bin)
            .arg("exec")
            .arg(&sandbox_name)
            .arg("--")
            .arg("curl")
            .arg("-s")
            .arg("--max-time")
            .arg("10")
            .arg("-o")
            .arg("/dev/null")
            .arg("-w")
            .arg("%{http_code}")
            .arg(format!("http://{host}:{port}/"))
            .output()
            .unwrap();
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    let allowed = curl_status(allowed_port, "127.0.0.3");
    let denied = curl_status(denied_port, "127.0.0.4");

    drop(host_holder);
    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);

    assert!(
        bind_out.contains("BOUND"),
        "the sandbox must bind its declared port in its own namespace even while the \
         host holds the same number — without isolation this is EADDRINUSE. Got: {bind_out}"
    );
    assert!(
        allowed.contains("200"),
        "an isolated sandbox must still reach an allowlisted host through the proxy. \
         This is the whole point: the namespace has no route to the host, so this \
         traffic went keeper relay -> unix socket -> host proxy -> upstream. A failure \
         here means that path is broken, not that the allowlist is wrong. Got: {allowed}"
    );
    assert!(
        !denied.contains("200"),
        "a host outside `network.allow` must still be refused — isolation must not \
         have turned the proxy into an open relay. Got: {denied}"
    );
}

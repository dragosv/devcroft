//! `add-egress-proxy`, end to end (tasks.md section 5): a real `up`, a
//! real keeper, and a real `curl` inside the sandbox — not just the
//! proxy's own unit tests, which never go through Landlock or a real
//! session. See `tests/lifecycle_hooks.rs` for why this needs
//! `CARGO_BIN_EXE_devcroft` and why each such test lives in its own
//! file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;

/// Accepts one connection and replies with a fixed 200 OK — enough for
/// `curl -w '%{http_code}'` to report, without pulling in a real HTTP
/// server crate for a one-shot fixture.
fn serve_one_ok_response(listener: TcpListener) {
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf); // drain the request
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
}

#[test]
fn network_allow_actually_filters_by_host_through_a_real_curl() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
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
        std::env::temp_dir().join(format!("devcroft-egress-proxy-e2e-{}", std::process::id()));
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
    // own-policy-baseline excludes host toolchain access — `curl` has to
    // come from the resolved environment, same reasoning
    // `tests/exec_up.rs` already documents for `bash`/`coreutils`.
    let install = Command::new("flox")
        .args(["install", "curl"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install curl failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        return;
    }

    // Two mock upstreams on two different loopback-family addresses: the
    // manifest allows exactly one by name, so a name-based (not merely
    // reachability-based) decision is what's actually under test.
    let allowed_listener = TcpListener::bind("127.0.0.3:0").unwrap();
    let allowed_port = allowed_listener.local_addr().unwrap().port();
    serve_one_ok_response(allowed_listener);

    let denied_listener = TcpListener::bind("127.0.0.4:0").unwrap();
    let denied_port = denied_listener.local_addr().unwrap().port();
    serve_one_ok_response(denied_listener);

    let sandbox_name = format!("e2eegress{}", std::process::id());
    // `127.0.0.3`, not `127.0.0.1`: `up_process` sets `NO_PROXY` to
    // exempt `localhost`/`127.0.0.1`/`::1` from proxying at all (so an
    // ordinary `network.ports`-granted dev server stays reachable
    // without an allowlist entry) — using `.1` here would make curl skip
    // the proxy entirely and hit Landlock's direct-connect denial
    // instead of the proxy's own host decision, testing the wrong thing.
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n\n[network]\ndefault = \"deny\"\nallow = [\"127.0.0.3\"]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let curl_status = |port: u16, host: &str| -> String {
        let out = Command::new(devcroft_bin)
            .arg("exec")
            .arg(&sandbox_name)
            .arg("--")
            .arg("curl")
            .arg("-s")
            .arg("--max-time")
            .arg("5")
            .arg("-o")
            .arg("/dev/null")
            .arg("-w")
            .arg("%{http_code}")
            .arg(format!("http://{host}:{port}/"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    let allowed_status = curl_status(allowed_port, "127.0.0.3");
    assert_eq!(
        allowed_status, "200",
        "network.allow = [\"127.0.0.3\"] must let a request to 127.0.0.3 through the proxy"
    );

    let denied_status = curl_status(denied_port, "127.0.0.4");
    assert_eq!(
        denied_status, "502",
        "127.0.0.4 is not on network.allow and must be refused by the proxy, not merely \
         unreachable — a 502 (not a connection error/timeout) proves the proxy itself decided"
    );

    let log = std::fs::read_to_string(&paths.proxy_log).unwrap_or_default();
    assert!(
        log.contains("allow") && log.contains(&allowed_port.to_string()),
        "proxy log must record the allow decision, log was: {log}"
    );
    assert!(
        log.contains("refuse") && log.contains("127.0.0.4"),
        "proxy log must record the refusal naming the denied host, log was: {log}"
    );

    let _ = down(&sandbox_name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

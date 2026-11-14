//! `network.loopback = "any"` (`allow-loopback-ephemeral-ports`), end to end
//! against a real sandbox.
//!
//! The key exists for programs that listen on a port the kernel picks and
//! then connect to it, Antigravity's `agy` first among them (its language
//! server binds `127.0.0.1:0`). Under it, devcroft installs no Landlock TCP
//! filter at all, and the sandbox's route-less network namespace is the
//! boundary (design.md L1). So this file checks two things:
//!
//! - the program runs with the key, and is refused without it;
//! - nothing outside the namespace becomes reachable: not an outside
//!   address, not a listener on the host's loopback, not a domain the
//!   allowlist does not name.
//!
//! Linux only: macOS refuses the key at `up`
//! (`lifecycle::up::tests::loopback_any_is_refused_at_backend_on_macos`).
//!
//! **Teeth-check (task 4.4), measured 2026-11-14.** With `up`'s refusals
//! bypassed and `isolate_network` forced off, so that `"any"` ran without a
//! namespace, this test failed at the outside-address assertion:
//! `connect(1.1.1.1:80)` *succeeded*, straight past the proxy, because with
//! no Landlock TCP filter and no namespace nothing stands in the way. That
//! is worse than the host-loopback case it was expected to fail on, and it
//! is exactly why `"any"` is refused rather than degraded wherever the
//! namespace is missing (design.md L3).

#![cfg(target_os = "linux")]

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::process::Command;

/// Run `script` with the row's `python3` inside `sandbox`, returning stdout
/// and stderr together.
fn python(devcroft_bin: &str, sandbox: &str, script: &str) -> String {
    let out = Command::new(devcroft_bin)
        .args(["exec", sandbox, "--", "python3", "-c", script])
        .output()
        .unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `agy`'s pattern: bind a port the kernel chooses, then connect to it.
const EPHEMERAL: &str = "\
import socket
s = socket.socket()
try:
    s.bind(('127.0.0.1', 0)); s.listen(1)
except OSError as e:
    print('BIND-DENIED', e); raise SystemExit
port = s.getsockname()[1]
try:
    socket.create_connection(('127.0.0.1', port), 2)
    print('BOTH-OK', port)
except OSError as e:
    print('CONNECT-DENIED', e)
";

/// Writes `extra` under `[network]` into the row's manifest and brings the
/// sandbox up from scratch.
fn up_with(fx: &dyn common::ProviderFixture, extra: &str) -> String {
    let sandbox_name = fx.sandbox_name().to_owned();
    let manifest = fx.project_root().join("devcroft.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        format!("{text}\n[network]\ndefault = \"deny\"\nallow = [\"example.com\"]\n{extra}"),
    )
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);
    assert_eq!(
        fx.bring_up(&UpOptions::default())
            .unwrap_or_else(|e| panic!("row {}: up failed: {e}", fx.name())),
        UpOutcome::Started
    );
    sandbox_name
}

fn usable(fx: &dyn common::ProviderFixture) -> bool {
    if !(fx.capabilities().network_probes && fx.capabilities().cli_drivable) {
        eprintln!(
            "row {}: skipped (needs python3 and a CLI-drivable row)",
            fx.name()
        );
        return false;
    }
    true
}

#[test]
fn the_key_lets_a_program_use_a_port_the_kernel_chooses_and_nothing_beyond() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let bin = env!("CARGO_BIN_EXE_devcroft");

    // Spec: "The same program without the key".
    for_each_row("lbdecl", |fx| {
        if !usable(fx) {
            return;
        }
        let name = up_with(fx, "");
        let out = python(bin, &name, EPHEMERAL);
        assert!(
            out.contains("BIND-DENIED"),
            "row {}: without the key a kernel-chosen port must be refused, got: {out}",
            fx.name()
        );
        down(&name).unwrap();
    });

    for_each_row("lbany", |fx| {
        if !usable(fx) {
            return;
        }
        let name = up_with(fx, "loopback = \"any\"\n");

        // Spec: "A program listens on a port the kernel chooses".
        let out = python(bin, &name, EPHEMERAL);
        assert!(
            out.contains("BOTH-OK"),
            "row {}: with the key, bind :0 and the connect to it must both work, got: {out}",
            fx.name()
        );

        // Spec: "A direct connection to an outside address".
        let out = python(
            bin,
            &name,
            "import socket\n\
             try:\n\
             \x20   socket.create_connection(('1.1.1.1', 80), 3); print('REACHED')\n\
             except OSError as e:\n\
             \x20   print('REFUSED', e)\n",
        );
        assert!(
            out.contains("REFUSED"),
            "row {}: an outside address must stay unreachable, got: {out}",
            fx.name()
        );

        // Spec: "A service on the host's loopback". A listener this test
        // owns, on the host's loopback, which the sandbox must not reach.
        let host = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host_port = host.local_addr().unwrap().port();
        let out = python(
            bin,
            &name,
            &format!(
                "import socket\n\
                 try:\n\
                 \x20   socket.create_connection(('127.0.0.1', {host_port}), 3); print('REACHED')\n\
                 except OSError as e:\n\
                 \x20   print('REFUSED', e)\n"
            ),
        );
        assert!(
            out.contains("REFUSED"),
            "row {}: a listener on the host's loopback (port {host_port}) must not be \
             reachable from the sandbox's own, got: {out}",
            fx.name()
        );
        drop(host);

        // Spec: "A destination not on the allowlist". Through the proxy the
        // sandbox is handed, credentials and all, so the refusal is the
        // allowlist's and not a missing token's: devcroft's proxy answers an
        // allowlist refusal with `502` naming the host, and a token problem
        // with `407` (`proxy::server`), so the two cannot be confused.
        let out = python(
            bin,
            &name,
            "import os, socket, base64, urllib.parse\n\
             u = urllib.parse.urlsplit(os.environ['HTTPS_PROXY'])\n\
             s = socket.create_connection((u.hostname, u.port), 5)\n\
             auth = ''\n\
             if u.username:\n\
             \x20   cred = urllib.parse.unquote(u.username) + ':' + urllib.parse.unquote(u.password or '')\n\
             \x20   auth = 'Proxy-Authorization: Basic ' + base64.b64encode(cred.encode()).decode() + '\\r\\n'\n\
             s.sendall(('CONNECT not-allowed.invalid:443 HTTP/1.1\\r\\nHost: not-allowed.invalid:443\\r\\n' + auth + '\\r\\n').encode())\n\
             data = b''\n\
             while True:\n\
             \x20   chunk = s.recv(4096)\n\
             \x20   if not chunk: break\n\
             \x20   data += chunk\n\
             print('RESPONSE', data.decode(errors='replace'))\n",
        );
        assert!(
            out.contains("RESPONSE HTTP/1.1 502") && out.contains("not-allowed.invalid"),
            "row {}: a domain off the allowlist must be refused by the proxy (502, naming the \
             host), got: {out}",
            fx.name()
        );

        // Visible in the rendered policy, with its origin.
        let render = Command::new(bin)
            .args(["policy", "--render"])
            .current_dir(fx.project_root())
            .output()
            .unwrap();
        let render = String::from_utf8_lossy(&render.stdout);
        assert!(
            render.contains("network.loopback") && render.contains("manifest:network.loopback"),
            "row {}: the rule must render with its origin, got: {render}",
            fx.name()
        );

        down(&name).unwrap();
    });
}

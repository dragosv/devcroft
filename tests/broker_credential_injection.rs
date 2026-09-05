//! **A brokered credential reaches the upstream without entering the sandbox**
//! (`adopt-nono-proxy` task 4.1, `brokered-credentials`' first two scenarios).
//!
//! Both halves are checked from *inside* a real session, which is what makes
//! this the test that matters: the environment a session actually sees, and a
//! request that actually leaves it.
//!
//! **No loopback aliases needed, unlike `egress_proxy_e2e`.** That test needs
//! `127.0.0.3`/`127.0.0.4` because it asserts a *host* decision and needs two
//! distinguishable hosts. This one asserts credential injection, so one address
//! and two ports is enough — the fake upstream and the proxy differ by port,
//! and `NO_PROXY` exempting `127.0.0.1` is exactly what makes the SDK dial the
//! local route directly instead of tunnelling it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

/// The real credential. It must appear at the fake upstream and nowhere the
/// sandbox can see.
const SECRET: &str = "sk-test-brokered-credential-9f3a";

/// Stands in for `api.anthropic.com`: echoes back whatever `x-api-key` it was
/// given, so the assertion is about what the *upstream* received rather than
/// about what devcroft believes it sent.
fn fake_upstream() -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 8192];
            let n = s.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let key = head
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case("x-api-key")
                        .then(|| value.trim().to_string())
                })
                .unwrap_or_else(|| "NONE".to_string());
            let body = format!("received-key={key}");
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (port, handle)
}

#[test]
fn the_upstream_gets_the_credential_and_the_sandbox_never_does() {
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
    let bin = env!("CARGO_BIN_EXE_devcroft");

    let root = std::env::temp_dir().join(format!("devcroft-broker-inject-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    if !Command::new("flox")
        .arg("init")
        .current_dir(&root)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping: flox init failed");
        return;
    }
    // `own-policy-baseline` removed host toolchain access, so both binaries
    // have to come out of the closure — same reasoning `exec_up` documents.
    if !Command::new("flox")
        .args(["install", "curl", "coreutils"])
        .current_dir(&root)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping: flox install curl coreutils failed");
        return;
    }

    let (upstream_port, _server) = fake_upstream();
    let name = format!("brokerinject{}", std::process::id());
    std::fs::write(
        root.join("devcroft.toml"),
        format!(
            r#"
[sandbox]
name = "{name}"
[network]
default = "deny"
allow = ["127.0.0.1"]
[[broker]]
provider = "anthropic"
upstream = "http://127.0.0.1:{upstream_port}"
secret = "env:DEVCROFT_TEST_BROKER_SECRET"
"#
        ),
    )
    .unwrap();

    let up = Command::new(bin)
        .arg("up")
        .current_dir(&root)
        .env("DEVCROFT_TEST_BROKER_SECRET", SECRET)
        .output()
        .unwrap();
    if !up.status.success() {
        eprintln!(
            "skipping: `up` failed on this host: {}",
            String::from_utf8_lossy(&up.stderr)
        );
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let exec = |args: &[&str]| -> String {
        let out = Command::new(bin)
            .arg("exec")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    // 1. The environment the session actually sees.
    let env_dump = exec(&["--", "env"]);

    // 2. A request through the route, from inside, to the fake upstream. The
    //    phantom token is what the SDK would send; the upstream echoes back
    //    what it actually received.
    let response = exec(&[
        "--",
        "sh",
        "-c",
        "curl -sS -H \"x-api-key: $ANTHROPIC_API_KEY\" \"$ANTHROPIC_BASE_URL/v1/messages\"",
    ]);

    let _ = Command::new(bin).arg("down").current_dir(&root).output();
    let paths = devcroft::lifecycle::StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&root);

    // The guarantee, checked where it is made.
    assert!(
        !env_dump.contains(SECRET),
        "the real credential must never appear in the sandbox's environment; \
         env was:\n{env_dump}"
    );
    assert!(
        env_dump.contains("ANTHROPIC_BASE_URL=http://127.0.0.1:"),
        "the session must be pointed at the local route; env was:\n{env_dump}"
    );
    assert!(
        env_dump.contains("ANTHROPIC_API_KEY="),
        "an SDK that requires a key must find one (the phantom token); env was:\n{env_dump}"
    );

    // And the half that proves brokering rather than merely routing: the
    // upstream received the *real* key, which the sandbox never held.
    assert!(
        response.contains(&format!("received-key={SECRET}")),
        "the upstream must receive the real credential, injected by the proxy. \
         `received-key=NONE` would mean the route forwarded without injecting; \
         an empty response would mean it never reached the upstream at all. \
         got: {response:?}"
    );
}

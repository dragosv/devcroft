//! **Where a granted port actually lives**, asserted from the host.
//!
//! This file exists because a regression shipped without it. When
//! `wants_network_isolation` started giving a sandbox its own network
//! namespace, a dev server inside it stopped answering on the host's
//! `127.0.0.1:<port>` — the classic "run it and open localhost in a
//! browser" workflow, broken for exactly the `default = "deny"` config
//! devcroft recommends. Nothing caught it:
//!
//! - `tests/network_ports_listen.rs` probes binding from *inside* the
//!   sandbox via `devcroft exec`.
//! - `tests/services_e2e.rs` asserts `host_process_count(..) > 0` — that
//!   the process is running, which is true either way.
//!
//! Both were satisfied while the property a user cares about was gone.
//! The lesson generalises past this bug: a test that only ever looks from
//! inside the boundary cannot see a change in what the boundary exposes.
//!
//! What is asserted here is the *current, intended* behaviour, not a
//! wish: an isolated sandbox's ports are namespace-local and reachable
//! through `ssh -L`. If a future change adds host-side port mapping
//! (`add-linux-agent-fleet` D8), the first assertion below flips and this
//! comment is the record of why it read the other way.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::Duration;

/// Deliberately high and unusual, so a stray host listener cannot be
/// mistaken for the sandbox's own.
const PORT: u16 = 18447;

fn host_can_reach(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(750),
    )
    .is_ok()
}

/// Starts a listener *inside* the sandbox and waits for it to be
/// reachable from inside, so the test never races the server's startup.
/// Returns the still-running child; the caller kills it.
fn serve_inside(devcroft_bin: &str, sandbox: &str, port: u16) -> Option<Child> {
    let child = Command::new(devcroft_bin)
        .arg("exec")
        .arg(sandbox)
        .arg("--")
        .arg("python3")
        .arg("-m")
        .arg("http.server")
        .arg(port.to_string())
        .arg("--bind")
        .arg("127.0.0.1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Readiness is checked from inside, since from the host is exactly
    // what is under test and would beg the question.
    for _ in 0..40 {
        let probe = Command::new(devcroft_bin)
            .arg("exec")
            .arg(sandbox)
            .arg("--")
            .arg("python3")
            .arg("-c")
            .arg(format!(
                "import socket,sys\n\
                 s=socket.socket()\n\
                 s.settimeout(0.5)\n\
                 sys.exit(0 if s.connect_ex(('127.0.0.1',{port}))==0 else 1)\n"
            ))
            .output()
            .ok()?;
        if probe.status.success() {
            return Some(child);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Some(child)
}

fn flox_project_with_python(tag: &str) -> Option<std::path::PathBuf> {
    if Command::new("flox").arg("--version").output().is_err() {
        return None;
    }
    let root =
        std::env::temp_dir().join(format!("devcroft-hostreach-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).ok()?;
    if !Command::new("flox")
        .arg("init")
        .current_dir(&root)
        .output()
        .ok()?
        .status
        .success()
    {
        return None;
    }
    if !Command::new("flox")
        .args(["install", "python3"])
        .current_dir(&root)
        .output()
        .ok()?
        .status
        .success()
    {
        return None;
    }
    Some(root)
}

#[test]
fn an_isolated_sandboxs_granted_port_is_namespace_local_not_host_visible() {
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
    let Some(project_root) = flox_project_with_python("iso") else {
        eprintln!("skipping: flox unavailable or environment setup failed");
        return;
    };

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    // Nothing may already hold the port, or the assertion below cannot
    // distinguish "the sandbox is isolated" from "something else answered".
    if host_can_reach(PORT) {
        eprintln!("skipping: something on this host already holds {PORT}");
        let _ = std::fs::remove_dir_all(&project_root);
        return;
    }

    // `default = "deny"` + a declared port: the shape that triggers
    // isolation, and the shape devcroft's own docs recommend.
    let sandbox_name = format!("e2ehriso{}", std::process::id());
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n\
         [network]\ndefault = \"deny\"\nports = [{PORT}]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let server = serve_inside(devcroft_bin, &sandbox_name, PORT);
    let reachable_from_host = host_can_reach(PORT);

    if let Some(mut c) = server {
        let _ = c.kill();
        let _ = c.wait();
    }
    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);

    assert!(
        !reachable_from_host,
        "an isolated sandbox's port answered on the host's own loopback. That is not \
         a failure of isolation being *wanted* — it means the sandbox did not get its \
         own namespace, so two sandboxes of one project would collide on this port \
         again. If host-side port mapping was deliberately added (fleet D8), this \
         assertion is the thing to update, together with `up`'s note and \
         `policy --render`'s `network.namespace` line."
    );
}

/// The other half, and the reason the first assertion is not simply
/// "ports don't work": without isolation the same manifest shape *is*
/// host-visible. Two tests rather than one because a single test asserting
/// only unreachability would pass just as well against a sandbox whose
/// server never started.
#[test]
fn a_non_isolated_sandboxs_granted_port_is_reachable_from_the_host() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let Some(project_root) = flox_project_with_python("plain") else {
        eprintln!("skipping: flox unavailable or environment setup failed");
        return;
    };

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    // A distinct port from the isolated test: cargo runs test binaries
    // concurrently, and both of these care about the *host's* namespace,
    // where sharing a number is interference rather than a subject.
    const PLAIN_PORT: u16 = 18448;
    if host_can_reach(PLAIN_PORT) {
        eprintln!("skipping: something on this host already holds {PLAIN_PORT}");
        let _ = std::fs::remove_dir_all(&project_root);
        return;
    }

    // `default = "allow"` leaves `network_block` false, so
    // `wants_network_isolation` is false and the sandbox shares the
    // host's port table — the pre-isolation behaviour, still current for
    // this manifest shape.
    let sandbox_name = format!("e2ehrplain{}", std::process::id());
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n\
         [network]\ndefault = \"allow\"\nports = [{PLAIN_PORT}]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let server = serve_inside(devcroft_bin, &sandbox_name, PLAIN_PORT);
    let reachable_from_host = host_can_reach(PLAIN_PORT);

    if let Some(mut c) = server {
        let _ = c.kill();
        let _ = c.wait();
    }
    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);

    assert!(
        reachable_from_host,
        "a sandbox sharing the host's port table must be reachable on it — this is \
         what makes the isolated case's unreachability a property of the namespace \
         rather than of the test never starting a server"
    );
}

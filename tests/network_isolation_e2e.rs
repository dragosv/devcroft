//! `CompiledPolicy::wants_network_isolation`, end to end against two real
//! sandboxes: the fix for two sandboxes both declaring the same service
//! port (README's own "Why" — every git worktree of a repo commits the
//! same `devcroft.toml`, so two of them starting Postgres on 5432 used to
//! collide with `EADDRINUSE`).
//!
//! The mechanism itself — a real network namespace, loopback brought up —
//! was already proven in isolation by `tests/fleet_netns.rs`. This test
//! proves the *wiring*: that `up` actually enters that namespace for a
//! qualifying sandbox, not just that the primitive works when called
//! directly.
//!
//! See `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// A python one-liner that binds `port`, prints `BOUND` the moment it
/// succeeds, then blocks forever accepting nothing — held open so a
/// *second* sandbox's own bind on the same number has something real to
/// collide with if the two share one port table.
fn hold_probe_script(port: u16) -> String {
    format!(
        "import socket, sys\n\
         s = socket.socket()\n\
         s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n\
         try:\n\
         \x20   s.bind(('127.0.0.1', {port}))\n\
         \x20   s.listen(1)\n\
         \x20   print('BOUND', flush=True)\n\
         except Exception as e:\n\
         \x20   print('DENIED', e, flush=True)\n\
         \x20   sys.exit(0)\n\
         s.accept()\n"
    )
}

/// Same bind attempt, but reports and exits immediately rather than
/// holding the socket — this is the sandbox whose isolation is actually
/// under test.
fn probe_once(devcroft_bin: &str, sandbox: &str, port: u16) -> String {
    let out = Command::new(devcroft_bin)
        .arg("exec")
        .arg(sandbox)
        .arg("--")
        .arg("python3")
        .arg("-c")
        .arg(format!(
            "import socket\n\
             s = socket.socket()\n\
             s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n\
             try:\n\
             \x20   s.bind(('127.0.0.1', {port}))\n\
             \x20   s.listen(1)\n\
             \x20   print('BOUND')\n\
             except Exception as e:\n\
             \x20   print('DENIED', e)\n"
        ))
        .output()
        .unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn two_sandboxes_bind_the_same_declared_port_without_colliding() {
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
    if Command::new("flox").arg("--version").output().is_err() {
        eprintln!("skipping: flox not on PATH");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    // One shared project root, flox environment installed once — two
    // sandboxes of the *same* project, exactly the git-worktree shape
    // the collision is about. `StatePaths` are keyed on sandbox name, not
    // project root, so this needs no directory duplication.
    let project_root = std::env::temp_dir().join(format!(
        "devcroft-network-isolation-e2e-{}",
        std::process::id()
    ));
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
        .args(["install", "python3"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install python3 failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        return;
    }

    // Deliberately high and unusual, so a stray host listener is not
    // mistaken for either sandbox successfully binding.
    const SHARED_PORT: u16 = 18225;

    let sandbox_a = format!("e2eisoa{}", std::process::id());
    let sandbox_b = format!("e2eisob{}", std::process::id());
    let manifest_for = |name: &str| {
        parse(&format!(
            "[sandbox]\nname = {name:?}\n\
             [network]\ndefault = \"deny\"\nports = [{SHARED_PORT}]\n"
        ))
        .unwrap()
        .0
    };
    let paths_a = StatePaths::new(&sandbox_a).unwrap();
    let paths_b = StatePaths::new(&sandbox_b).unwrap();
    let _ = std::fs::remove_dir_all(&paths_a.root);
    let _ = std::fs::remove_dir_all(&paths_b.root);

    assert_eq!(
        up(
            &manifest_for(&sandbox_a),
            &project_root,
            &UpOptions::default()
        )
        .unwrap(),
        UpOutcome::Started
    );
    assert_eq!(
        up(
            &manifest_for(&sandbox_b),
            &project_root,
            &UpOptions::default()
        )
        .unwrap(),
        UpOutcome::Started
    );

    // Sandbox A holds the port open for the duration of the test — this
    // is the collision opportunity. Without a private network namespace
    // per sandbox, B's own bind on the identical number would get
    // `EADDRINUSE` from the kernel, since both would share one port
    // table.
    let mut holder = Command::new(devcroft_bin)
        .arg("exec")
        .arg(&sandbox_a)
        .arg("--")
        .arg("python3")
        .arg("-c")
        .arg(hold_probe_script(SHARED_PORT))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut a_ready_line = String::new();
    BufReader::new(holder.stdout.take().unwrap())
        .read_line(&mut a_ready_line)
        .unwrap();
    assert!(
        a_ready_line.contains("BOUND"),
        "sandbox A must bind {SHARED_PORT} first, got: {a_ready_line}"
    );

    // Sandbox B binds the *same* number while A still holds it.
    let b_result = probe_once(devcroft_bin, &sandbox_b, SHARED_PORT);

    let _ = holder.kill();
    let _ = holder.wait();

    assert!(
        b_result.contains("BOUND"),
        "sandbox B must bind {SHARED_PORT} even while sandbox A holds the identical \
         number open — a shared port table would deny this with EADDRINUSE, which is \
         exactly the collision devcroft's per-sandbox network namespace exists to \
         prevent. Got: {b_result}"
    );

    down(&sandbox_a).unwrap();
    down(&sandbox_b).unwrap();
    let _ = std::fs::remove_dir_all(&paths_a.root);
    let _ = std::fs::remove_dir_all(&paths_b.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

/// The other half of `wants_network_isolation`'s contract: a sandbox that
/// wants *any* egress must not be isolated, since an isolated namespace
/// cannot reach the host-bound egress proxy at all. Verified by the
/// existing `tests/egress_proxy_e2e.rs` continuing to pass unmodified —
/// noted here as a compile-time reminder of why this test does not also
/// cover that case, so a later reader does not mistake the omission for
/// an oversight.
#[allow(dead_code)]
fn scope_note() {}

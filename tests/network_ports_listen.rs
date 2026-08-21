//! `network.ports`, end to end against a real sandbox: the manifest key
//! that lets a sandbox bind a loopback listener *without* giving up
//! egress filtering.
//!
//! This closes what the README and `add-flox-services`' proposal both
//! described as a gap in the policy model itself — "no outbound access,
//! but I can still run my dev server" being inexpressible. That
//! description was wrong, and this test is the correction: nono's
//! profile schema has always carried the field (`open_port`), and
//! devcroft simply never emitted it. The only workaround used to be
//! `network.default = "allow"`, which restores binding by dropping
//! egress filtering entirely — precisely backwards for the sandbox
//! population that most needs both.
//!
//! `open_port` rather than the adjacent `listen_port` was settled
//! empirically, not from the schema's field descriptions: against nono
//! 0.71.0 on Linux, a profile with `block: true` plus `open_port` binds
//! `127.0.0.1` successfully, while `listen_port` granted neither a
//! loopback nor a `0.0.0.0` bind. `assert_ungranted_port_denied` below
//! is what keeps that from silently regressing into a blanket unlock:
//! nono ignores unknown profile fields rather than rejecting them, so a
//! renamed field would grant nothing while still looking configured.
//!
//! See `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::process::Command;

/// A python one-liner reporting whether binding `port` on loopback
/// succeeds. `python3` must be installed into the sandbox's flox
/// environment (`Sandbox::new`'s `flox install python3`) — before own-
/// policy-baseline this comment claimed it "comes from the flox-
/// provisioned environment" while actually resolving through the host's
/// `/usr/bin` via the now-excluded `system_read_linux_core` group, which
/// is exactly the smuggled host passthrough that change targets.
fn bind_probe(devcroft_bin: &str, sandbox: &str, port: u16) -> String {
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
fn a_granted_port_binds_while_egress_stays_denied_and_other_ports_do_not() {
    if Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
    {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-network-ports-e2e-{}", std::process::id()));
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
    // own-policy-baseline excludes host toolchain access, so `python3`
    // must come from the flox closure, not (as the comment on
    // `bind_probe` already assumed) the host.
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
    // mistaken for the sandbox successfully binding.
    const GRANTED: u16 = 18123;
    const UNGRANTED: u16 = 18124;

    let sandbox_name = format!("e2eports{}", std::process::id());
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n\
         [network]\ndefault = \"deny\"\nports = [{GRANTED}]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // The grant works under a deny-all egress policy — the combination
    // that was previously impossible to express.
    let granted = bind_probe(devcroft_bin, &sandbox_name, GRANTED);
    assert!(
        granted.contains("BOUND"),
        "a granted port must bind under network.default = deny, got: {granted}"
    );

    // ...and it is an allowlist, not a blanket unlock. Without this the
    // test would still pass if `ports` accidentally disabled port
    // mediation altogether.
    let ungranted = bind_probe(devcroft_bin, &sandbox_name, UNGRANTED);
    assert!(
        ungranted.contains("DENIED"),
        "an ungranted port must still be denied, got: {ungranted}"
    );

    // Egress remains filtered: granting a local port must not have
    // widened outbound access, which is the entire reason this key
    // exists rather than telling users to set `default = "allow"`.
    // `policy --render` re-parses the manifest from disk and must run
    // from within the project, so the file has to exist here: `up` above
    // took an already-parsed `Manifest`, as every test in this suite does.
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = {sandbox_name:?}\n\
             [network]\ndefault = \"deny\"\nports = [{GRANTED}]\n"
        ),
    )
    .unwrap();
    let render = Command::new(devcroft_bin)
        .arg("policy")
        .arg("--render")
        .current_dir(&project_root)
        .output()
        .unwrap();
    let render = String::from_utf8_lossy(&render.stdout);
    assert!(
        render.contains("network.block: true"),
        "egress must stay blocked, got: {render}"
    );
    // The "nothing reaches the backend that --render cannot show"
    // invariant: the port is in profile.json, so it must be here too.
    assert!(
        render.contains(&GRANTED.to_string()) && render.contains("manifest:network.ports"),
        "the granted port must be visible with its origin, got: {render}"
    );

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

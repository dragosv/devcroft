//! **A brokered route whose credential is absent fails `up`, not the agent's
//! first request** (`adopt-nono-proxy` tasks 3.2/4.3, `brokered-credentials`).
//!
//! Deferred, that failure surfaces as an upstream authentication error from
//! inside a sandbox — the least diagnosable place it could appear, and one that
//! reads as the agent's fault rather than as a missing export on the host.
//!
//! This test needs no provider and no sandbox: resolution runs before the
//! health decision and before provider resolution precisely so nothing is
//! started first, and that ordering is what the second assertion checks.

use std::process::Command;

fn project(dir: &std::path::Path, manifest: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("devcroft.toml"), manifest).unwrap();
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("devcroft-broker-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

const MANIFEST: &str = r#"
[sandbox]
name = "brokerprobe"
[network]
allow = ["api.anthropic.com"]
[[broker]]
provider = "anthropic"
upstream = "https://api.anthropic.com"
secret = "env:DEVCROFT_TEST_BROKER_SECRET_UNSET"
"#;

#[test]
fn a_missing_broker_credential_fails_up_at_layer_provider() {
    let root = scratch("missing");
    project(&root, MANIFEST);

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("up")
        .current_dir(&root)
        .env_remove("DEVCROFT_TEST_BROKER_SECRET_UNSET")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    // Exit 3 is the error contract's environment/provider code. Asserted
    // rather than "non-zero": a config-layer 2 here would mean devcroft had
    // classified a missing host credential as a malformed manifest, which
    // sends the user to edit the wrong thing.
    assert_eq!(
        out.status.code(),
        Some(3),
        "a missing brokered credential is an environment precondition (exit 3), \
         not a manifest error. stderr: {stderr}"
    );
    assert!(
        stderr.contains("anthropic"),
        "the failure must name the route the user has to fix, got: {stderr}"
    );
    assert!(
        stderr.contains("DEVCROFT_TEST_BROKER_SECRET_UNSET"),
        "and name the variable that is missing, or the user is told a route is \
         broken without being told why. got: {stderr}"
    );

    // Nothing may be left behind: the spec requires no sandbox running, and a
    // half-created state dir would make the next `up` adopt a sandbox that was
    // never brought up.
    let state = devcroft::lifecycle::StatePaths::new("brokerprobe").unwrap();
    assert!(
        !state.pidfile.exists(),
        "a refused `up` must leave no keeper pidfile behind"
    );

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&state.root);
}

/// The control. Without it, a `resolve_brokers` that refused *every* route —
/// or one that never ran — would be indistinguishable from a working check.
#[test]
fn a_present_credential_gets_past_the_broker_check() {
    let root = scratch("present");
    project(&root, MANIFEST);

    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("up")
        .current_dir(&root)
        .env("DEVCROFT_TEST_BROKER_SECRET_UNSET", "not-a-real-key")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    // `up` may still fail here — this host may have no usable provider — but
    // it must fail *somewhere else*. The broker check is what is under test,
    // not the whole lifecycle.
    assert!(
        !stderr.contains("cannot be brokered"),
        "a credential that is present must clear the broker check; failing here \
         would mean the check refuses everything. stderr: {stderr}"
    );

    let state = devcroft::lifecycle::StatePaths::new("brokerprobe").unwrap();
    let _ = devcroft::lifecycle::down("brokerprobe");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&state.root);
}

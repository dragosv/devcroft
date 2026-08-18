//! Provider-declared services (add-flox-services), end to end against a
//! real flox environment: a service declared in the flox manifest is
//! started inside the sandbox by the keeper, and — the part that
//! actually matters — is *gone from the host* after `down`.
//!
//! Teardown is asserted by observing process absence, never by trusting
//! a stop command's exit status, because the failure mode this guards
//! against was reproduced by hand during development: killing
//! process-compose alone (SIGTERM to that pid) left its spawned child
//! running and holding the port. What makes teardown correct here is
//! that the keeper registers process-compose in the same registry
//! sessions use, and the shutdown handler terminates each registered
//! *process group* — so the child goes with it.
//!
//! Requires `process-compose` in the project's environment, which is
//! also devcroft's own requirement: `up` fails at layer `provider` when
//! services are declared and the binary is not a closure member, rather
//! than starting a sandbox whose services silently never come up.
//!
//! See `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, up};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// Deliberately unusual, so a stray host listener cannot be mistaken for
/// the sandbox's own service.
const PORT: u16 = 18777;

fn tooling_missing() -> bool {
    Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
}

/// Counts host processes whose argv contains `needle`, without matching
/// the counting command itself — `pgrep -f` matches its own command line
/// and reports a false positive, which bit repeatedly during development.
fn host_process_count(needle: &str) -> usize {
    let out = Command::new("ps").arg("-eo").arg("args").output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(needle))
        .count()
}

fn service_responds(devcroft_bin: &str, sandbox: &str) -> bool {
    Command::new(devcroft_bin)
        .arg("exec")
        .arg(sandbox)
        .arg("--")
        .arg("python3")
        .arg("-c")
        .arg(format!(
            "import socket,sys\n\
             s = socket.socket()\n\
             s.settimeout(2)\n\
             sys.exit(0 if s.connect_ex(('127.0.0.1', {PORT})) == 0 else 1)\n"
        ))
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_declared_service_runs_inside_the_sandbox_and_is_reaped_by_down() {
    if tooling_missing() {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-services-e2e-{}", std::process::id()));
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

    // `process-compose` is devcroft's runtime requirement for services;
    // `python3` provides the service itself. Installing both is slow, so
    // a failure here skips rather than fails — same posture as the other
    // real-tooling tests.
    let install = Command::new("flox")
        .arg("install")
        .arg("process-compose")
        .arg("python3")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !install.status.success() {
        eprintln!(
            "skipping: flox install failed: {}",
            String::from_utf8_lossy(&install.stderr)
        );
        let _ = std::fs::remove_dir_all(&project_root);
        return;
    }

    // Declare the service the documented way, with its port arriving
    // through `vars` — the shape that would silently start on the wrong
    // port if `vars` were dropped.
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    // flox's generated manifest already contains an (empty) `[services]`
    // table, so appending a second header would be a duplicate key —
    // which devcroft's own parser correctly rejects. Extend the existing
    // section instead.
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest = manifest.replacen(
        "[services]\n",
        &format!(
            "[services]\nweb.command = \"python3 -m http.server $WEB_PORT --bind 127.0.0.1\"\n\
             web.vars.WEB_PORT = \"{PORT}\"\n"
        ),
        1,
    );
    std::fs::write(&manifest_path, manifest).unwrap();

    let sandbox_name = format!("e2esvc{}", std::process::id());
    let (dc_manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n\
         [network]\ndefault = \"deny\"\nports = [{PORT}]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&dc_manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // devcroft generated its own config — not flox's service-config.yaml.
    assert!(
        devcroft::services::config_path(&project_root).is_file(),
        "up must write the generated process-compose config"
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut up_ok = false;
    while Instant::now() < deadline {
        if service_responds(devcroft_bin, &sandbox_name) {
            up_ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(
        up_ok,
        "the declared service must be running inside the sandbox; log: {}",
        std::fs::read_to_string(devcroft::services::log_path(&project_root))
            .unwrap_or_else(|_| "(no log)".into())
    );

    assert!(
        host_process_count("http.server") > 0,
        "sanity: the service process should be visible on the host before teardown"
    );

    down(&sandbox_name).unwrap();

    // The real assertion. Poll briefly: teardown escalates SIGTERM to
    // SIGKILL after a grace period, so "gone" is not instantaneous.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut reaped = false;
    while Instant::now() < deadline {
        if host_process_count("http.server") == 0 && host_process_count("process-compose up") == 0 {
            reaped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(
        reaped,
        "down must leave no service process behind — neither process-compose nor the \
         service it spawned"
    );

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn services_without_process_compose_fail_at_the_provider_layer() {
    if tooling_missing() {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root =
        std::env::temp_dir().join(format!("devcroft-services-nopc-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    let init = Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!("skipping: flox init failed");
        return;
    }

    // Services declared, but the environment deliberately does not
    // provide process-compose.
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest = manifest.replacen("[services]\n", "[services]\nweb.command = \"true\"\n", 1);
    std::fs::write(&manifest_path, manifest).unwrap();

    let sandbox_name = format!("e2esvcnopc{}", std::process::id());
    let (dc_manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let err = up(&dc_manifest, &project_root, &UpOptions::default())
        .expect_err("declaring services without process-compose must fail `up`");
    let msg = err.to_string();
    assert!(
        msg.starts_with("provider:"),
        "must fail at layer `provider`, got: {msg}"
    );
    assert!(
        msg.contains("process-compose"),
        "the error must name what is missing, got: {msg}"
    );

    // Nothing should have been left running or written.
    assert!(
        !Path::new(&devcroft::services::config_path(&project_root)).exists(),
        "a failed precondition must not leave a generated config behind"
    );

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

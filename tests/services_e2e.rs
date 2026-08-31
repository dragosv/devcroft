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
    !devcroft::policy::backend_supported()
        || (Command::new("flox").arg("--version").output().is_err()
            || !devcroft::provider::host_can_build_nix_closures())
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
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
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
    // `python3` provides the service itself; `bash` supplies the `sh`
    // process-compose's generated config now names explicitly
    // (services/mod.rs's `shell_command`) instead of its own unreachable
    // `/usr/bin/bash` default. Installing all three is slow, so a
    // failure here skips rather than fails — same posture as the other
    // real-tooling tests.
    let install = Command::new("flox")
        .arg("install")
        .arg("process-compose")
        .arg("python3")
        .arg("bash")
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
        devcroft::services::config_path(&project_root, &sandbox_name).is_file(),
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
        std::fs::read_to_string(devcroft::services::log_path(&project_root, &sandbox_name))
            .unwrap_or_else(|_| "(no log)".into())
    );

    assert!(
        host_process_count("http.server") > 0,
        "sanity: the service process should be visible on the host before teardown"
    );

    // Observability: a running service is reported per-service, by name.
    let status = devcroft::lifecycle::status(&dc_manifest).unwrap();
    let report = status
        .services
        .as_ref()
        .expect("a sandbox with a running service must report service state");
    assert_eq!(report.supervisor_error, None);
    assert_eq!(report.states.len(), 1);
    assert_eq!(report.states[0].name, "web");
    assert_eq!(
        report.states[0].health,
        devcroft::services::ServiceHealth::Running
    );

    // ...and a *dead* one is reported as failed rather than vanishing.
    // This is the case the `services` spec's "failure is visible, never
    // silent" exists for, and the one decision 3's no-auto-restart
    // rationale depends on. It only works because process-compose is run
    // with `--keep-project`: without it, process-compose exits once its
    // last service is gone, taking the API socket — and the only record
    // of why — with it.
    let pid = report.states[0]
        .pid
        .expect("a running service reports its pid");
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut reported_failed = false;
    while Instant::now() < deadline {
        let st = devcroft::lifecycle::status(&dc_manifest).unwrap();
        if let Some(report) = st.services.as_ref()
            && report.states.iter().any(|s| s.health.is_failure())
        {
            reported_failed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(
        reported_failed,
        "a killed service must surface as failed, not disappear from status"
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

    // With the supervisor now gone but the sandbox's recorded
    // declarations intact, `status` must still account for the declared
    // service rather than reporting nothing. This is the regression the
    // `services` spec's "SHALL NOT be omitted from service listings"
    // forbids: before reconciliation, a dead supervisor and a sandbox
    // that never declared anything produced byte-identical output.
    let after = devcroft::lifecycle::status(&dc_manifest).unwrap();
    let report = after.services.as_ref().expect(
        "a sandbox that declared services must not report none once the supervisor is gone",
    );
    assert!(
        report.supervisor_error.is_some(),
        "an unreachable supervisor must be named, not silently omitted"
    );
    assert!(
        report.states.iter().any(|s| s.name == "web"),
        "the declared service must still be listed, got: {:?}",
        report.states
    );

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn services_without_process_compose_fail_at_the_provider_layer() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
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
        !Path::new(&devcroft::services::config_path(
            &project_root,
            &sandbox_name
        ))
        .exists(),
        "a failed precondition must not leave a generated config behind"
    );

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

/// Task 2.4, in the only shape that can actually happen.
///
/// The literal spec reading — a manifest asking for services under a
/// provider with none — is unreachable: declarations come from the
/// *provider's* manifest and `devcroft.toml` has no `[services]` of its
/// own, so a nix project has no way to ask. The reachable variant, and
/// the one users hit, is a project carrying a flox environment whose
/// services are declared while `env.provider` says `nix`. Those were
/// silently ignored: the sandbox came up reporting no services at all,
/// indistinguishable from a project declaring none — exactly the silent
/// failure the whole `services` spec is written against.
///
/// Needs both flox (to declare the services) and nix (to resolve the
/// provider that will not run them). The check is driven by the
/// *resolved* `ServiceSupport`, not by a guess from the provider's name,
/// so resolution has to succeed first — which is what makes it work
/// unchanged for any future provider reporting `Unsupported`, devbox
/// included, rather than being a nix special case.
#[test]
fn services_declared_for_another_provider_fail_rather_than_being_ignored() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }
    if Command::new("nix").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        eprintln!("skipping: nix not on PATH");
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-services-wrongprovider-e2e-{}",
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
        eprintln!("skipping: flox init failed");
        return;
    }

    // Into the manifest's *existing* `[services]` table, not appended as
    // a second one — flox's stock manifest already ships a commented
    // `[services]` section, and a duplicate table makes the whole file
    // fail to parse, which would make this test pass for the wrong
    // reason (no declarations found, rather than the check firing).
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest = manifest.replacen("[services]\n", "[services]\nweb.command = \"true\"\n", 1);
    std::fs::write(&manifest_path, manifest).unwrap();

    // A real, minimal flake so the nix provider actually resolves — the
    // check under test runs after resolution, so without this the test
    // would pass on "no nix environment found" and prove nothing.
    // Systems enumerated statically for the reason `provider::nix`'s own
    // fixtures document: flakes evaluate pure, and `builtins.currentSystem`
    // does not exist under pure evaluation.
    std::fs::write(
        project_root.join("flake.nix"),
        r#"{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
    in { devShells = builtins.listToAttrs (map (s: {
         name = s; value.default = (import nixpkgs { system = s; }).mkShell {};
       }) systems); };
}
"#,
    )
    .unwrap();
    if !Command::new("nix")
        .arg("flake")
        .arg("lock")
        .arg(&project_root)
        .output()
        .unwrap()
        .status
        .success()
    {
        eprintln!("skipping: nix flake lock failed (likely no network for nixpkgs)");
        return;
    }

    let sandbox_name = format!("e2esvcwrongprov{}", std::process::id());
    let (dc_manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n\n[env]\nprovider = \"nix\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let err = up(&dc_manifest, &project_root, &UpOptions::default())
        .expect_err("services declared for a provider with none must fail `up`");
    let msg = err.to_string();
    assert!(
        msg.starts_with("provider:"),
        "must fail at layer `provider`, got: {msg}"
    );
    assert!(
        msg.contains("web"),
        "the error must name the service that would be ignored, got: {msg}"
    );
    assert!(
        msg.contains("nix"),
        "the error must name the provider that cannot supply it, got: {msg}"
    );

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

/// `--skip-hooks` promises that nothing project-supplied runs. Refusing
/// to come up because of services that would not have started anyway
/// would make the escape hatch useless in exactly the situation it
/// exists for — debugging a broken environment.
#[test]
fn skip_hooks_bypasses_the_wrong_provider_service_check() {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-services-skiphooks-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    if !Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap()
        .status
        .success()
    {
        eprintln!("skipping: flox init failed");
        return;
    }
    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest = manifest.replacen("[services]\n", "[services]\nweb.command = \"true\"\n", 1);
    std::fs::write(&manifest_path, manifest).unwrap();

    let sandbox_name = format!("e2esvcskiphooks{}", std::process::id());
    // `provider = "flox"`, so this exercises the skip path without
    // needing a resolvable flake — the check under test is the one in
    // `prepare_services`, which runs for every provider.
    let (dc_manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let outcome = up(
        &dc_manifest,
        &project_root,
        &UpOptions {
            skip_hooks: true,
            ..UpOptions::default()
        },
    );
    assert!(
        matches!(outcome, Ok(UpOutcome::Started)),
        "--skip-hooks must still bring the sandbox up, got: {outcome:?}"
    );

    let _ = down(&sandbox_name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

/// Task 2.6: `policy --render` is byte-identical with and without
/// services declared.
///
/// The property holds by construction — `policy::compile` takes only the
/// devcroft `Manifest`, and service declarations live in the *provider's*
/// manifest, never reaching it. That is exactly why it is worth pinning:
/// the proposal's "No change to policy compilation" is a promise that a
/// future change threading service ports into the policy would quietly
/// break, and a service asking for a port is supposed to ask the manifest
/// for it, not devcroft for an exemption.
///
/// Runs before any `up`, so it needs neither `process-compose` nor a
/// resolvable environment — `policy --render` compiles from the manifest.
#[test]
fn policy_render_is_unchanged_by_declaring_services() {
    if Command::new("flox").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return;
    }

    let project_root = std::env::temp_dir().join(format!(
        "devcroft-services-policyparity-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    if !Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap()
        .status
        .success()
    {
        eprintln!("skipping: flox init failed");
        return;
    }
    let sandbox_name = format!("e2esvcpolparity{}", std::process::id());
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!("[sandbox]\nname = {sandbox_name:?}\n"),
    )
    .unwrap();

    let render = || {
        let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
            .args(["policy", "--render"])
            .current_dir(&project_root)
            .output()
            .unwrap();
        assert!(out.status.success(), "policy --render failed: {out:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    let without_services = render();

    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    std::fs::write(
        &manifest_path,
        manifest.replacen(
            "[services]\n",
            "[services]\nweb.command = \"python3 -m http.server 8099\"\n",
            1,
        ),
    )
    .unwrap();

    let with_services = render();

    assert_eq!(
        without_services, with_services,
        "declaring a service must not change the compiled policy by so much as a byte"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

/// Sets up a real flox project declaring `services_toml` inside the
/// stock manifest's existing `[services]` table, with `process-compose`
/// (devcroft's own requirement) plus `bash` (the `sh` the generated
/// config names) and `python3` installed.
///
/// Returns `None` when the tooling is unavailable or an install fails —
/// the same skip-rather-than-fail posture every other real-tooling test
/// in this repo uses.
fn flox_project_declaring(tag: &str, services_toml: &str) -> Option<std::path::PathBuf> {
    if tooling_missing() {
        eprintln!("skipping: no usable flox here (not on PATH, or no reachable Nix store)");
        return None;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    let project_root =
        std::env::temp_dir().join(format!("devcroft-services-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    if !Command::new("flox")
        .arg("init")
        .current_dir(&project_root)
        .output()
        .unwrap()
        .status
        .success()
    {
        eprintln!("skipping: flox init failed");
        return None;
    }
    if !Command::new("flox")
        .args(["install", "process-compose", "python3", "bash"])
        .current_dir(&project_root)
        .output()
        .unwrap()
        .status
        .success()
    {
        eprintln!("skipping: flox install failed");
        let _ = std::fs::remove_dir_all(&project_root);
        return None;
    }

    let manifest_path = project_root.join(".flox/env/manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    std::fs::write(
        &manifest_path,
        manifest.replacen("[services]\n", &format!("[services]\n{services_toml}"), 1),
    )
    .unwrap();

    Some(project_root)
}

/// Polls `status` until `pred` holds for the service report, or gives up.
fn wait_for_service_report(
    manifest: &devcroft::config::Manifest,
    pred: impl Fn(&devcroft::services::ServicesReport) -> bool,
) -> Option<devcroft::services::ServicesReport> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(st) = devcroft::lifecycle::status(manifest)
            && let Some(report) = st.services
            && pred(&report)
        {
            return Some(report);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

/// Task 6.3 — a service whose command exits non-zero **at startup**.
///
/// Distinct from the killed-while-running case the main test above
/// covers: the `services` spec requires failed-at-start and exited-later
/// to be distinguishable, and this is the state that used to be
/// indistinguishable from "never declared".
#[test]
fn a_service_that_exits_non_zero_at_startup_is_reported_failed_and_the_sandbox_stays_usable() {
    let Some(project_root) = flox_project_declaring(
        "failstart",
        "boom.command = \"sh -c 'echo service-said-boom >&2; exit 3'\"\n",
    ) else {
        return;
    };

    let sandbox_name = format!("e2esvcfail{}", std::process::id());
    let (dc_manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    // The `services` spec's "Services do not block sandbox availability":
    // `up` succeeds even though the service will not.
    assert_eq!(
        up(&dc_manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let report = wait_for_service_report(&dc_manifest, |r| {
        r.states.iter().any(|s| s.health.is_failure())
    })
    .unwrap_or_else(|| {
        panic!(
            "a service exiting non-zero at startup must be reported as failed; log: {}",
            std::fs::read_to_string(devcroft::services::log_path(&project_root, &sandbox_name))
                .unwrap_or_else(|_| "(no log)".into())
        )
    });
    assert!(
        report.states.iter().any(|s| s.name == "boom"),
        "the failed service must be listed by name, got: {:?}",
        report.states
    );

    // "with its log tail reachable" — the reason has to be findable, not
    // merely the fact of failure.
    let log = std::fs::read_to_string(devcroft::services::log_path(&project_root, &sandbox_name))
        .unwrap_or_default();
    assert!(
        log.contains("service-said-boom"),
        "the service's own output must be reachable through the service log, got: {log}"
    );

    // ...and the sandbox is still usable despite it.
    let exec = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(["exec", &sandbox_name, "--", "sh", "-c", "echo alive"])
        .current_dir(&project_root)
        .output()
        .unwrap();
    assert!(
        exec.status.success() && String::from_utf8_lossy(&exec.stdout).contains("alive"),
        "a failed service must not make the sandbox unusable, got: {exec:?}"
    );

    let _ = down(&sandbox_name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

/// Task 6.4 — a service denied a port by `[network]` fails visibly, with
/// the same denial any in-sandbox process would get.
///
/// The "same denial" half is asserted directly rather than assumed: the
/// identical bind is attempted through `exec`, and both must fail. That
/// is the proposal's "a service that needs a port is asking the manifest
/// for it, not asking devcroft for an exemption" made checkable.
#[test]
fn a_service_denied_its_port_fails_the_same_way_any_session_would() {
    const UNGRANTED: u16 = 18991;

    let Some(project_root) = flox_project_declaring(
        "denyport",
        &format!("listener.command = \"python3 -m http.server {UNGRANTED} --bind 127.0.0.1\"\n"),
    ) else {
        return;
    };

    let sandbox_name = format!("e2esvcdeny{}", std::process::id());
    // Deny-default with NO ports granted: the service's bind is exactly
    // the operation the policy refuses.
    let (dc_manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n[network]\ndefault = \"deny\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&dc_manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let report = wait_for_service_report(&dc_manifest, |r| {
        r.states.iter().any(|s| s.health.is_failure())
    })
    .unwrap_or_else(|| {
        panic!(
            "a service denied its port must surface as failed, not as healthy; log: {}",
            std::fs::read_to_string(devcroft::services::log_path(&project_root, &sandbox_name))
                .unwrap_or_else(|_| "(no log)".into())
        )
    });
    assert!(
        report.states.iter().any(|s| s.name == "listener"),
        "the denied service must be listed by name, got: {:?}",
        report.states
    );

    // The same operation, from an ordinary session, must be denied
    // identically — the service got no exemption and no extra penalty.
    let exec = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args([
            "exec",
            &sandbox_name,
            "--",
            "python3",
            "-c",
            &format!(
                "import socket,sys\n\
                 s = socket.socket()\n\
                 s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n\
                 try:\n\
                 \x20   s.bind(('127.0.0.1', {UNGRANTED}))\n\
                 except OSError:\n\
                 \x20   sys.exit(7)\n\
                 sys.exit(0)\n"
            ),
        ])
        .current_dir(&project_root)
        .output()
        .unwrap();
    assert_eq!(
        exec.status.code(),
        Some(7),
        "an ordinary session must be denied the same ungranted port, got: {exec:?}"
    );

    let _ = down(&sandbox_name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

/// Task 3.6 (process tier) — a service that **ignores SIGTERM** is still
/// gone after teardown.
///
/// The distinction that matters: `down` returning success proves nothing
/// about whether anything died. A service trapping SIGTERM survives the
/// polite half of teardown, so this exercises the escalation to SIGKILL
/// after the grace period, and asserts the outcome by process absence —
/// never by a stop command's exit status, which is the failure this
/// whole file was written against.

#[test]
fn a_service_ignoring_sigterm_is_still_gone_after_teardown() {
    // Unique per run, not a constant: a constant marker makes runs
    // alias each other, so a single orphan left by an earlier *failing*
    // run is counted by every later run and keeps failing it long after
    // the bug is fixed. That happened while developing this test, and
    // cost a full debugging cycle chasing a fix that already worked.
    let marker = format!("devcroft-sigterm-refusenik-{}", std::process::id());

    let Some(project_root) = flox_project_declaring(
        "sigterm",
        &format!(
            "stubborn.command = \"sh -c 'trap \\\"\\\" TERM; while true; do sleep 1; done' {marker}\"\n"
        ),
    ) else {
        return;
    };

    let sandbox_name = format!("e2esvcsigterm{}", std::process::id());
    let (dc_manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&dc_manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // Sanity: it has to actually be running, or "gone after down" is
    // vacuous.
    let running = wait_for_service_report(&dc_manifest, |r| {
        r.states
            .iter()
            .any(|s| s.health == devcroft::services::ServiceHealth::Running)
    });
    assert!(
        running.is_some() && host_process_count(&marker) > 0,
        "the SIGTERM-ignoring service must be running before teardown is meaningful; log: {}",
        std::fs::read_to_string(devcroft::services::log_path(&project_root, &sandbox_name))
            .unwrap_or_else(|_| "(no log)".into())
    );

    down(&sandbox_name).unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut reaped = false;
    while Instant::now() < deadline {
        if host_process_count(&marker) == 0 {
            reaped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(
        reaped,
        "a service that ignores SIGTERM must still be gone after teardown — \
         escalation to SIGKILL is what makes `down` a guarantee rather than a request"
    );

    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

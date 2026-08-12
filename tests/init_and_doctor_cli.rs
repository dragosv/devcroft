//! `devcroft init` and `devcroft doctor` (cli spec's "init"/"doctor"
//! requirements, task 7.1), through the real built binary. `init` needs
//! no `nono`/`flox` (it only touches the filesystem); `doctor` shells out
//! to both, so its assertions are conditional on them actually being
//! installed, same as every other real-tooling e2e test in this suite.

use std::path::PathBuf;
use std::process::Command;

fn scratch_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "devcroft-init-doctor-cli-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

#[test]
fn init_generates_a_manifest_that_parses_with_no_warnings() {
    let dir = scratch_project("basic");

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let manifest_path = dir.join("devcroft.toml");
    assert!(manifest_path.exists());

    let text = std::fs::read_to_string(&manifest_path).unwrap();
    let (manifest, warnings) = devcroft::config::parse(&text).unwrap();
    assert!(
        warnings.is_empty(),
        "a freshly generated manifest should never itself trigger a warning, got {warnings:?}"
    );
    assert_eq!(manifest.env.provider, "flox");
    assert!(!manifest.sandbox.name.is_empty());

    assert!(
        String::from_utf8_lossy(&out.stdout).contains("run `flox init` before `devcroft up`"),
        "no .flox/ present should point at `flox init`"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_slugifies_the_directory_name() {
    let parent = scratch_project("slug");
    let dir = parent.join("My_Weird Project Name!!");
    std::fs::create_dir_all(&dir).unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert!(
        devcroft::config::is_valid_name(&manifest.sandbox.name),
        "generated name {:?} must itself be a valid sandbox name",
        manifest.sandbox.name
    );

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let dir = scratch_project("noforce");

    assert!(run(&dir, &["init"]).status.success());
    let first = std::fs::read_to_string(dir.join("devcroft.toml")).unwrap();

    let out = run(&dir, &["init"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--force"),
        "the error should mention --force"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("devcroft.toml")).unwrap(),
        first,
        "a rejected init must not touch the existing manifest"
    );

    let out = run(&dir, &["init", "--force"]);
    assert!(out.status.success());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_detects_an_existing_flox_environment() {
    let dir = scratch_project("flox");
    std::fs::create_dir_all(dir.join(".flox")).unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ready for `devcroft up`"),
        "an existing .flox/ should skip the flox-init advice"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_advises_on_a_pinned_rust_toolchain_without_flox() {
    let dir = scratch_project("rust-pin");
    std::fs::write(
        dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rust-toolchain.toml"));
    assert!(stdout.contains("rustup alone"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_advises_on_a_pinned_node_version_without_flox() {
    let dir = scratch_project("nvmrc");
    std::fs::write(dir.join(".nvmrc"), "20\n").unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(".nvmrc"));
    assert!(stdout.contains("nvm alone"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_advises_on_a_pinned_python_version_without_flox() {
    let dir = scratch_project("pyversion");
    std::fs::write(dir.join(".python-version"), "3.12\n").unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(".python-version"));
    assert!(stdout.contains("pyenv alone"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_prefers_an_existing_flox_environment_over_a_toolchain_pin() {
    let dir = scratch_project("flox-over-pin");
    std::fs::create_dir_all(dir.join(".flox")).unwrap();
    std::fs::write(
        dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ready for `devcroft up`"),
        "an existing .flox/ should win over a toolchain pin, got {stdout:?}"
    );
    assert!(
        !stdout.contains("rustup alone"),
        "pin advice should not print when .flox/ already exists, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doctor_reports_backend_and_provider_when_installed() {
    if Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
    {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    let dir = scratch_project("doctor");
    let out = run(&dir, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("[PASS] backend: nono"));
    assert!(stdout.contains("[PASS] kernel:"));
    assert!(stdout.contains("[PASS] provider: flox"));
    assert!(
        stdout.contains("no devcroft.toml found from here"),
        "an empty scratch dir has no manifest to check degradation for"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doctor_reports_manifest_degradation_when_one_is_discoverable() {
    if Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
    {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    let dir = scratch_project("doctor-manifest");
    assert!(run(&dir, &["init"]).status.success());

    let out = run(&dir, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("[PASS] manifest:") || stdout.contains("[WARN] manifest:"),
        "a discoverable manifest should produce a manifest-degradation line, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

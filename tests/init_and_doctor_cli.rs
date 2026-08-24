//! `devcroft init` and `devcroft doctor` (cli spec's "init"/"doctor"
//! requirements, task 7.1), through the real built binary. `init` needs
//! no external tooling at all (it only touches the filesystem).
//!
//! `doctor` probes the kernel for Landlock support and shells out to
//! whichever provider the discovered manifest declares — so a test gates
//! only on what its own assertions need, never on the full set. A test
//! asserting about nix does not gate on flox, and vice versa: that
//! asymmetry is the behaviour two of these tests exist to pin down.

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

/// These tests are about the backend/provider/manifest lines specifically,
/// not about every independent check `doctor` runs. `gvisor-backend` is
/// legitimately, per the `cli` delta spec's own "runsc present but
/// platform unusable" scenario, `[FAIL]` rather than `[WARN]` on a host
/// where `runsc` is installed but its platform doesn't actually work —
/// true of this repo's own devcontainer once task group 8 installed
/// `runsc` into it (rootless gVisor needs a user-namespace creation this
/// container's default seccomp profile denies; see
/// `.devcontainer/devcontainer.json`'s own comment on that). Asserting
/// blanket `doctor` success would make these tests depend on a platform
/// capability they have nothing to do with, so this only fails on a
/// `[FAIL]` line the test doesn't already expect.
fn assert_no_unexpected_doctor_failures(stdout: &str) {
    // `provider: nix` joins `gvisor-backend` as a tolerated failure for
    // the same reason: both are optional capabilities whose absence says
    // nothing about the backend/provider/manifest lines these tests are
    // actually about. nix earns its place here only now that `doctor`
    // probes flakes with a real evaluation — it used to probe with `nix
    // flake --help`, which succeeds even when the experimental feature
    // is off, so this line could never fire and the tests passed on a
    // host where `up` would have failed.
    let unexpected: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            l.starts_with("[FAIL]")
                && !l.starts_with("[FAIL] gvisor-backend")
                && !l.starts_with("[FAIL] provider: nix")
                // Same reasoning as nix: devbox's own probe depends on
                // Nix being usable too, an independent capability these
                // tests aren't about.
                && !l.starts_with("[FAIL] provider: devbox")
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected doctor failures: {unexpected:?}\nfull output: {stdout}"
    );
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
fn init_detects_an_existing_nix_flake() {
    let dir = scratch_project("flake");
    std::fs::write(
        dir.join("flake.nix"),
        "{ description = \"x\"; outputs = { self }: {}; }",
    )
    .unwrap();
    std::fs::write(dir.join("flake.lock"), "{}").unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(manifest.env.provider, "nix");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ready for `devcroft up`"),
        "a flake with flake.lock should be ready, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_on_a_flake_without_lock_advises_locking_it() {
    let dir = scratch_project("flake-nolock");
    std::fs::write(
        dir.join("flake.nix"),
        "{ description = \"x\"; outputs = { self }: {}; }",
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(manifest.env.provider, "nix");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("nix flake lock"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_prefers_flox_over_a_flake_when_both_are_present() {
    let dir = scratch_project("flox-and-flake");
    std::fs::create_dir_all(dir.join(".flox")).unwrap();
    std::fs::write(
        dir.join("flake.nix"),
        "{ description = \"x\"; outputs = { self }: {}; }",
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(
        manifest.env.provider, "flox",
        "flox must win when both a flox environment and a flake are present"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ready for `devcroft up`"));
    assert!(
        stdout.contains("flake.nix") && stdout.contains("provider = \"nix\""),
        "should note the flake was also found and is available, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_prefers_an_existing_flake_over_a_toolchain_pin() {
    let dir = scratch_project("flake-over-pin");
    std::fs::write(
        dir.join("flake.nix"),
        "{ description = \"x\"; outputs = { self }: {}; }",
    )
    .unwrap();
    std::fs::write(dir.join("flake.lock"), "{}").unwrap();
    std::fs::write(
        dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("rustup alone"),
        "pin advice should not print when flake.nix already exists, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_detects_an_existing_devbox_project() {
    let dir = scratch_project("devbox");
    std::fs::write(
        dir.join("devbox.json"),
        r#"{"packages": ["ripgrep@latest"]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("devbox.lock"),
        r#"{"packages": {"ripgrep@latest": {}}}"#,
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(manifest.env.provider, "devbox");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ready for `devcroft up`"),
        "a devbox project with everything locked should be ready, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_on_a_devbox_project_with_unresolved_packages_advises_install() {
    let dir = scratch_project("devbox-unresolved");
    std::fs::write(
        dir.join("devbox.json"),
        r#"{"packages": ["ripgrep@latest"]}"#,
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(manifest.env.provider, "devbox");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("devbox install"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// Spec: "Init on a devbox project with nothing to resolve" — a project
/// declaring no packages is ready for `up` with no lock advice, matching
/// the `env-provider` rule that nothing declared means nothing to lock.
#[test]
fn init_on_a_devbox_project_with_nothing_to_resolve_is_ready() {
    let dir = scratch_project("devbox-empty");
    std::fs::write(dir.join("devbox.json"), "{}").unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ready for `devcroft up`"),
        "a devbox project declaring no packages has nothing to lock, got {stdout:?}"
    );
    assert!(
        !stdout.contains("devbox install"),
        "should not advise locking when nothing is declared, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_prefers_flox_over_devbox_when_both_are_present() {
    let dir = scratch_project("flox-and-devbox");
    std::fs::create_dir_all(dir.join(".flox")).unwrap();
    std::fs::write(dir.join("devbox.json"), r#"{"packages": []}"#).unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(
        manifest.env.provider, "flox",
        "flox must win when both a flox environment and a devbox project are present"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("devbox.json") && stdout.contains("provider = \"devbox\""),
        "should note the devbox project was also found and is available, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_prefers_devbox_over_a_flake_when_both_are_present() {
    let dir = scratch_project("devbox-and-flake");
    std::fs::write(dir.join("devbox.json"), r#"{"packages": []}"#).unwrap();
    std::fs::write(
        dir.join("flake.nix"),
        "{ description = \"x\"; outputs = { self }: {}; }",
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let (manifest, _) =
        devcroft::config::parse(&std::fs::read_to_string(dir.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(
        manifest.env.provider, "devbox",
        "devbox must win when both a devbox project and a bare flake are present"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("flake.nix") && stdout.contains("provider = \"nix\""),
        "should note the flake was also found and is available, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_prefers_an_existing_devbox_project_over_a_toolchain_pin() {
    let dir = scratch_project("devbox-over-pin");
    std::fs::write(dir.join("devbox.json"), r#"{"packages": []}"#).unwrap();
    std::fs::write(
        dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    )
    .unwrap();

    let out = run(&dir, &["init"]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("rustup alone"),
        "pin advice should not print when devbox.json already exists, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_disambiguates_a_real_name_collision_across_projects() {
    if Command::new("flox").arg("--version").output().is_err() {
        eprintln!("skipping: flox not on PATH");
        return;
    }
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    // Two unrelated projects that happen to share a leaf directory name —
    // ~/proiect-A/api and ~/proiect-B/api from the report this test is
    // covering — so both would default to the same plain slug "api".
    let parent = scratch_project("collision");
    let project_a = parent.join("proiect-A").join("api");
    let project_b = parent.join("proiect-B").join("api");
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();
    for p in [&project_a, &project_b] {
        assert!(
            Command::new("flox")
                .arg("init")
                .current_dir(p)
                .output()
                .unwrap()
                .status
                .success()
        );
    }

    // First project: no prior state under "api" anywhere, so it gets the
    // plain slug, and `up` actually starts it (real state, real meta).
    assert!(run(&project_a, &["init"]).status.success());
    let (manifest_a, _) =
        devcroft::config::parse(&std::fs::read_to_string(project_a.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(manifest_a.sandbox.name, "api");
    let up_a = run(&project_a, &["up"]);
    assert!(up_a.status.success(), "{up_a:?}");

    // Second project: state for "api" now exists and belongs to project_a,
    // not project_b — a real collision, so init must disambiguate.
    let out_b = run(&project_b, &["init"]);
    assert!(out_b.status.success(), "{out_b:?}");
    let (manifest_b, _) =
        devcroft::config::parse(&std::fs::read_to_string(project_b.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_ne!(
        manifest_b.sandbox.name, "api",
        "a real collision must not silently reuse the colliding name"
    );
    assert!(
        manifest_b.sandbox.name.starts_with("api-"),
        "got {:?}",
        manifest_b.sandbox.name
    );
    assert!(devcroft::config::is_valid_name(&manifest_b.sandbox.name));

    // Re-running init in project_a itself (same project, not a collision)
    // must still keep the plain slug.
    assert!(run(&project_a, &["init", "--force"]).status.success());
    let (manifest_a2, _) =
        devcroft::config::parse(&std::fs::read_to_string(project_a.join("devcroft.toml")).unwrap())
            .unwrap();
    assert_eq!(manifest_a2.sandbox.name, "api");

    let _ = run(&project_a, &["rm", "--yes"]);
    let _ = std::fs::remove_dir_all(
        devcroft::lifecycle::StatePaths::new(&manifest_a.sandbox.name)
            .unwrap()
            .root,
    );
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn doctor_reports_backend_and_provider_when_installed() {
    // `nono` is no longer a runtime dependency of the backend check at all
    // (use-nono-library: it's a linked crate, not a `PATH` lookup). What
    // gates skipping is whether the kernel actually has Landlock — this
    // test asserts doctor prints `[PASS] backend:`, which it correctly
    // will not do on a host where the probe fails.
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if Command::new("flox").arg("--version").output().is_err() {
        eprintln!("skipping: flox not on PATH");
        return;
    }

    let dir = scratch_project("doctor");
    let out = run(&dir, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_no_unexpected_doctor_failures(&stdout);
    assert!(stdout.contains("[PASS] backend:"));
    assert!(stdout.contains("[PASS] provider: flox"));
    assert!(
        stdout.contains("no devcroft.toml found from here"),
        "an empty scratch dir has no manifest to check degradation for"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `doctor` checks the provider the project declares, and no others.
///
/// The regression: it used to probe `flox` unconditionally and `[FAIL]`
/// when absent, so a project with `provider = "nix"` was told its
/// environment was broken on a host that deliberately has no flox — and
/// because the checks were chained with `&&`, the nix probe that project
/// actually depends on never ran at all. Deliberately does **not** gate
/// on flox being installed: the whole point is that this project does
/// not need it.
#[test]
fn doctor_checks_only_the_provider_the_manifest_declares() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    let dir = scratch_project("doctor-nix-only");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"doctornixonly\"\n[env]\nprovider = \"nix\"\n",
    )
    .unwrap();

    let stdout = String::from_utf8_lossy(&run(&dir, &["doctor"]).stdout).into_owned();

    assert!(
        !stdout.contains("provider: flox"),
        "a nix project must not report on flox at all, got:\n{stdout}"
    );
    assert!(
        stdout.contains("provider: nix"),
        "a nix project must report on nix, got:\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The converse, and the case that must keep working: a flox project
/// reports flox and stays silent about nix.
#[test]
fn doctor_on_a_flox_project_does_not_report_nix() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    let dir = scratch_project("doctor-flox-only");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"doctorfloxonly\"\n[env]\nprovider = \"flox\"\n",
    )
    .unwrap();

    let stdout = String::from_utf8_lossy(&run(&dir, &["doctor"]).stdout).into_owned();

    assert!(
        !stdout.contains("provider: nix"),
        "a flox project must not report on nix at all, got:\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Task 4.2: a devbox project reports devbox and stays silent about flox
/// and nix — the same per-provider scoping the nix/flox pair above
/// already pins, now exercised for the third provider.
#[test]
fn doctor_on_a_devbox_project_reports_devbox_and_stays_silent_about_flox_and_nix() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }

    let dir = scratch_project("doctor-devbox-only");
    std::fs::write(
        dir.join("devcroft.toml"),
        "[sandbox]\nname = \"doctordevboxonly\"\n[env]\nprovider = \"devbox\"\n",
    )
    .unwrap();

    let stdout = String::from_utf8_lossy(&run(&dir, &["doctor"]).stdout).into_owned();

    assert!(
        !stdout.contains("provider: flox"),
        "a devbox project must not report on flox at all, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("provider: nix"),
        "a devbox project must not report on the nix *provider* line — devbox's own \
         Nix-usability probe reports under \"provider: devbox\", not \"provider: nix\", \
         got:\n{stdout}"
    );
    assert!(
        stdout.contains("provider: devbox"),
        "a devbox project must report on devbox, got:\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doctor_reports_nix_when_installed_with_flakes_enabled() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // No flox gate: this test asserts about nix, and `doctor` in a
    // directory with no manifest requires no provider at all.
    // Must match `doctor`'s own probe exactly, or this test skips on a
    // different condition than the one it asserts about. `nix flake
    // --help` — what this used to use — succeeds even with the
    // experimental feature disabled, so the test ran on hosts where
    // flakes were off and then asserted `doctor` said they were on.
    let Ok(nix_out) = Command::new("nix")
        .arg("eval")
        .arg("--expr")
        .arg("1")
        .output()
    else {
        eprintln!("skipping: nix not on PATH");
        return;
    };
    if !nix_out.status.success() {
        eprintln!("skipping: nix present but flakes not enabled on this host");
        return;
    }

    let dir = scratch_project("doctor-nix");
    let out = run(&dir, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_no_unexpected_doctor_failures(&stdout);
    assert!(
        stdout.contains("[PASS] provider: nix") && stdout.contains("flakes enabled"),
        "got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doctor_reports_manifest_degradation_when_one_is_discoverable() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    if Command::new("flox").arg("--version").output().is_err() {
        eprintln!("skipping: flox not on PATH");
        return;
    }

    let dir = scratch_project("doctor-manifest");
    assert!(run(&dir, &["init"]).status.success());

    let out = run(&dir, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_no_unexpected_doctor_failures(&stdout);
    assert!(
        stdout.contains("[PASS] manifest:") || stdout.contains("[WARN] manifest:"),
        "a discoverable manifest should produce a manifest-degradation line, got {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

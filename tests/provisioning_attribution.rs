//! `sandbox-provisioning` task group 3: when the activation script fails in
//! the provisioning process, `up` says whether the provisioning policy refused
//! it something and names what, or says the failure is the script's own.
//!
//! The denied hook reads a file in the real home, the way a hook reads
//! `~/.npmrc`; confined, it sees `No such file or directory` for a file the
//! host has. The broken hook fails with no help from the policy at all.
//! Driven through the provider seam, as `provisioning_confinement.rs` is.

#![cfg(feature = "test-support")]

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpError, UpOptions, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn store_path_with_a_shell() -> Option<PathBuf> {
    std::fs::read_dir("/nix/store")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("bin/sh").is_file() && p.to_string_lossy().contains("bash"))
}

struct Row {
    store: PathBuf,
    hook: String,
}

impl ProviderEntry for Row {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: BTreeMap::from([(
                "PATH".to_owned(),
                format!("{}:/usr/bin", self.store.join("bin").display()),
            )]),
            unset: Vec::new(),
            read_only_grants: vec!["/nix/store".to_owned(), "/usr".to_owned()],
            activation_script: Some(self.hook.clone()),
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }
    fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
        Ok("provisioning-attribution-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

/// `up` with `hook` as the activation script; the provisioning error's
/// message, and the log.
fn failing_up(tag: &str, hook: String) -> Option<(String, String)> {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return None;
    }
    let Some(store) = store_path_with_a_shell() else {
        eprintln!("skipping: no /nix/store path with a shell on this host");
        return None;
    };
    // SAFETY: each test sets the same value.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft")) };

    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcprovattr{tag}{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2eprovattr{tag}{pid}");
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let outcome = up_with_provider(
        &manifest,
        &project,
        &UpOptions::default(),
        &Row { store, hook },
    );
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let _ = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project);

    match outcome {
        Err(UpError::Provider(ProviderError::Provisioning(msg))) => Some((msg, log)),
        other => panic!("expected a provisioning failure, got {other:?}\nlog:\n{log}"),
    }
}

#[test]
fn a_hook_denied_a_path_it_needs_is_told_which_and_by_what() {
    let Some(real_home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("skipping: no HOME");
        return;
    };
    // The project lives under /tmp; a home there would be granted.
    if real_home.starts_with("/tmp") || !real_home.is_dir() {
        eprintln!(
            "skipping: HOME {} is not a separate home",
            real_home.display()
        );
        return;
    }
    // A config file only the real home holds, made by the test.
    let config = real_home.join(format!(
        ".devcroft-attribution-probe-{}",
        std::process::id()
    ));
    std::fs::write(&config, "registry=https://example.invalid\n").unwrap();
    let outcome = failing_up("denied", format!("cat {} || exit 1", config.display()));
    let _ = std::fs::remove_file(&config);
    let Some((msg, log)) = outcome else { return };

    assert!(
        msg.contains("the provisioning policy refused it"),
        "a denial must be called one: {msg}\nlog:\n{log}"
    );
    assert!(
        msg.contains(&format!("{}: not granted by any rule", config.display())),
        "the denied path and the rule must be named: {msg}"
    );
    // What the hook itself saw is kept, so the two can be matched up.
    assert!(msg.contains("No such file or directory"), "{msg}");
}

#[test]
fn a_hook_that_is_simply_broken_is_not_blamed_on_the_policy() {
    let Some((msg, log)) = failing_up(
        "broken",
        "echo 'settings.toml: unknown key `colour`' >&2; exit 4".to_owned(),
    ) else {
        return;
    };
    assert!(
        !msg.contains("the provisioning policy refused it"),
        "a broken hook must not read as a denial: {msg}\nlog:\n{log}"
    );
    assert!(msg.contains("exited with status 4"), "{msg}");
    assert!(msg.contains("the failure is the script's own"), "{msg}");
    // The provider's own error, as it printed it.
    assert!(msg.contains("unknown key `colour`"), "{msg}");
}

//! `sandbox-provisioning` 5.2 and 5.3: the provider's activation script runs
//! in the provisioning process, under the provisioning policy, and that
//! policy is what a hook meets when it reaches for the user's home. Its own
//! `HOME` is the sandbox's (`.devcroft/<name>/home`), so writing there does
//! not touch the real one; and a file in the real home is neither writable
//! nor readable, the way a credential would be.
//!
//! Driven through the provider seam, as `provisioning_network.rs` is: a row
//! whose closure is a store shell plus the host's `/usr`.

#![cfg(feature = "test-support")]

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
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
        Ok("provisioning-confinement-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

#[test]
fn a_hook_cannot_write_to_or_read_from_the_real_home() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let Some(store) = store_path_with_a_shell() else {
        eprintln!("skipping: no /nix/store path with a shell on this host");
        return;
    };
    let Some(real_home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("skipping: no HOME");
        return;
    };
    // The project lives under /tmp; a real home there would be inside what
    // the sandbox is granted, and the test would prove nothing.
    if real_home.starts_with("/tmp") || !real_home.is_dir() {
        eprintln!(
            "skipping: HOME {} is not a separate home",
            real_home.display()
        );
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft")) };

    let pid = std::process::id();
    // Stands in for a credential: a file only the real home holds, made by
    // the test, never by the program under test.
    let credential = real_home.join(format!(".devcroft-provision-probe-{pid}"));
    std::fs::write(&credential, "secret\n").unwrap();
    let escape = real_home.join(format!(".devcroft-provision-written-{pid}"));

    let project = PathBuf::from(format!("/tmp/dcprovhome{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2eprovhome{pid}");
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    // Each attempt records its own result in the project, which is writable.
    let row = Row {
        store,
        hook: format!(
            "echo cache > \"$HOME/hook-cache\"; \
             echo \"$HOME\" > hook-home; \
             if echo x > {escape} 2>/dev/null; then echo WROTE > hook-write; \
             else echo DENIED > hook-write; fi; \
             if read -r line < {credential} 2>/dev/null; then echo \"READ $line\" > hook-read; \
             else echo DENIED > hook-read; fi",
            escape = escape.display(),
            credential = credential.display(),
        ),
    };
    let outcome = up_with_provider(&manifest, &project, &UpOptions::default(), &row);
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let read = |f: &str| {
        std::fs::read_to_string(project.join(f))
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let (home, wrote, read_cred) = (read("hook-home"), read("hook-write"), read("hook-read"));
    let sandbox_home = devcroft::services::artifact_dir(&project, &name).join("home");
    let cached = sandbox_home.join("hook-cache").is_file();
    let escaped = escape.exists();
    let _ = std::fs::remove_file(&escape);
    let _ = std::fs::remove_file(&credential);
    let _ = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    // 5.2: the hook's home is the sandbox's, and what it writes there stays
    // there; the real home is not written.
    assert_eq!(home, sandbox_home.display().to_string(), "{log}");
    assert!(
        cached,
        "the hook's write to $HOME did not land in the sandbox's home\n{log}"
    );
    assert_eq!(wrote, "DENIED", "{log}");
    assert!(!escaped, "the hook wrote into the real home");
    // 5.3: a file in the real home is not readable from the hook.
    assert_eq!(
        read_cred, "DENIED",
        "the hook read a file outside its profile\n{log}"
    );
}

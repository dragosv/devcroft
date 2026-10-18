//! What an agent runtime looks up by name and the project never declares
//! (`add-agent-workload` 6.3, measured with Claude Code installed by npm
//! inside a flox sandbox): `sh` on `PATH` and a `SHELL` that exists, and the
//! CA bundle the provider's environment names, which the view lacked.
//!
//! A provider row whose `PATH` is nixpkgs' node alone, with no shell on it
//! (devcroft's comes from the closure), and whose `SSL_CERT_FILE` is this host's spelling, a link in
//! `/usr/lib` to a bundle in `/etc`, outside every grant.

#![cfg(feature = "test-support")]

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const SPELLED: &str = "/usr/lib/ssl/cert.pem";

/// The measured shape: a store `bin` with no `sh` in it (nixpkgs' node),
/// whose closure has one (bash), which is where devcroft's shell comes from.
fn closure_without_sh_on_path() -> Option<PathBuf> {
    std::fs::read_dir("/nix/store")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains("-nodejs-"))
        .filter(|p| p.join("bin").is_dir() && !p.join("bin/sh").exists())
        .find(|p| {
            Command::new("nix-store")
                .args(["--query", "--requisites"])
                .arg(p)
                .output()
                .is_ok_and(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .any(|q| Path::new(q).join("bin/sh").is_file())
                })
        })
}

struct Row {
    closure: PathBuf,
}

impl ProviderEntry for Row {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: BTreeMap::from([
                // Nothing named `sh` anywhere on it.
                (
                    "PATH".to_owned(),
                    self.closure.join("bin").display().to_string(),
                ),
                ("SSL_CERT_FILE".to_owned(), SPELLED.to_owned()),
            ]),
            unset: Vec::new(),
            read_only_grants: vec!["/nix/store".to_owned()],
            activation_script: None,
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }
    fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
        Ok("agent-runtime-env-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

#[test]
fn a_session_finds_sh_by_name_a_real_shell_and_the_ca_bundle() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let Some(closure) = closure_without_sh_on_path() else {
        eprintln!(
            "skipping: no nixpkgs node in /nix/store (or no nix-store) to stand in for a \
             closure whose PATH has no shell"
        );
        return;
    };
    let Ok(bundle) = std::fs::canonicalize(SPELLED) else {
        eprintln!("skipping: no {SPELLED} on this host");
        return;
    };
    let bundle = bundle.display().to_string();
    if bundle.starts_with("/nix/store") || bundle.starts_with("/usr/") {
        eprintln!("skipping: {SPELLED} resolves inside a grant here ({bundle})");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft")) };

    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcagentenv{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2eagentenv{pid}");
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let outcome = up_with_provider(&manifest, &project, &UpOptions::default(), &Row { closure });
    let devcroft = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_devcroft"))
            .current_dir(&project)
            .args(args)
            .output()
            .unwrap()
    };
    // `sh` by name, as npm's lifecycle scripts and an agent's shell tool ask.
    let session = devcroft(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "echo \"SHELL=$SHELL\"; echo \"CERT=$SSL_CERT_FILE\"; \
         if read -r line < \"$SSL_CERT_FILE\"; then echo CERT_READ; fi",
    ]);
    // What `policy --render` renders from, since this project has no
    // manifest on disk for it to find: the policy artifact `up` wrote, and
    // `Meta` for the reconstruction.
    let artifact = std::fs::read_to_string(devcroft::services::policy_artifact(&project, &name))
        .unwrap_or_default();
    let meta = devcroft::lifecycle::read_meta(&paths.meta).ok().flatten();
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let _ = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    let out = String::from_utf8_lossy(&session.stdout);
    let err = String::from_utf8_lossy(&session.stderr);
    assert!(session.status.success(), "{out}{err}\nlog:\n{log}");
    let meta = meta.expect("up records Meta");
    let shell = meta.shell.expect("up records the shell");
    assert!(out.contains(&format!("SHELL={shell}\n")), "{out}");
    // Pointed at where the grant is mounted, and readable there.
    assert!(out.contains(&format!("CERT={bundle}\n")), "{out}");
    assert!(
        out.contains("CERT_READ"),
        "the CA bundle is not readable inside: {out}{err}"
    );
    assert_eq!(meta.trust_store, std::slice::from_ref(&bundle));
    let policy: devcroft::policy::CompiledPolicy =
        serde_json::from_str(&artifact).unwrap_or_else(|e| panic!("{e}: {artifact}"));
    assert!(
        policy
            .filesystem_read
            .iter()
            .any(|g| g.value == bundle && g.origin == devcroft::policy::Origin::TrustStore),
        "{:?}",
        policy.filesystem_read
    );
}

//! **The seam is the enforcement path** (`provider-injection-seam`), proven
//! by driving a real `up` with a provider the manifest does not name.
//!
//! This is the verification `add-test-runtime-fixture` task 1.2 asks for:
//! not "the trait compiles", but that the provider entry points `up` uses
//! actually go *through* the injected row. Two of the three are observable
//! from outside `up` and are asserted below:
//!
//! - `resolve` — the sandbox comes up in the environment the row returned,
//!   read back out of `meta.json`'s recorded grants;
//! - `fingerprint` — the value the row produced is what `status` will later
//!   compare against, read back out of `meta.json`.
//!
//! **The third, `static_name`, is not assertable from out here**, and the
//! bottom of this file records why rather than shipping a check that cannot
//! fail — it was written, found to pass with the mechanism removed, and
//! replaced with the explanation.
//!
//! A seam covering only resolution would leave the other two dispatching on
//! `manifest.env.provider`, so an injected row would exercise a composition
//! production never produces. That is the failure this file exists to make
//! impossible rather than to watch for.
//!
//! **Why this row is store-backed and not synthetic.** `up` refuses an
//! environment whose shell is not inside the store — measured in this
//! change's task 0, where `shell::resolve` returned `None` for a real
//! `/bin/sh` and for a real `sh` copied outside the store, while a genuine
//! store path resolved. Until that guard is generalized (design.md D4), a
//! row that drives `up` at all must be store-backed, so this one borrows a
//! shell from the host's store rather than inventing one. That is a
//! constraint on the fixture, not a weakening of the seam.
#![cfg(feature = "test-support")]

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A store path that actually contains a POSIX shell, or `None`.
///
/// The same probe this change's task 0 used. `None` is a skip, not a
/// failure: a host with no usable store cannot run this, and that is a
/// property of the host (`provider::host_can_build_nix_closures` is the
/// project's standing precedent for probing the capability rather than the
/// binary).
fn store_path_with_a_shell() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/nix/store").ok()?;
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("bin/sh").is_file() || p.join("bin/bash").is_file())
}

/// The injected row. Deliberately minimal: it exists to prove the three
/// entry points are wired, not to be a usable fixture — that is group 2's
/// job.
struct BorrowedStoreRow {
    store: PathBuf,
    fingerprint: String,
}

/// The name this row reports itself as.
///
/// **A real provider's name, not an invented one**, because `static_name`
/// becomes `Origin::Provider` and surfaces in `policy --render` and `why` —
/// user-facing output of a shipped binary. `provider-injection-seam`
/// requires that no origin token exist which a real provider could not
/// emit, so a row borrows an existing name rather than adding a word to
/// that vocabulary.
const ROW_NAME: &str = "nix";

impl ProviderEntry for BorrowedStoreRow {
    fn resolve(&self, _project_root: &Path) -> Result<Resolution, ProviderError> {
        let mut env = BTreeMap::new();
        env.insert(
            "PATH".to_string(),
            self.store.join("bin").to_string_lossy().into_owned(),
        );
        Ok(Resolution {
            env,
            unset: Vec::new(),
            read_only_grants: vec![self.store.to_string_lossy().into_owned()],
            activation_script: None,
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }

    fn fingerprint(&self, _project_root: &Path) -> Result<String, ProviderError> {
        Ok(self.fingerprint.clone())
    }

    fn static_name(&self) -> &'static str {
        ROW_NAME
    }
}

#[test]
fn an_injected_row_drives_all_three_provider_entry_points() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let Some(store) = store_path_with_a_shell() else {
        eprintln!("skipping: no /nix/store path with a shell on this host");
        return;
    };

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    // Short and canonical, for the two reasons this project keeps
    // rediscovering: `sun_path` is 103 usable bytes, and macOS's `/tmp` is a
    // symlink whose un-canonicalized spelling a sandbox is refused
    // (docs/known-gaps.md).
    let project_root = PathBuf::from(format!("/tmp/dcseam{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    let project_root = project_root.canonicalize().unwrap();

    let sandbox_name = format!("e2eseam{}", std::process::id());
    // The manifest names `flox`. The row reports `nix`. Every assertion
    // below that sees `nix` is therefore seeing the *injected* row rather
    // than the manifest — which is exactly the property under test, and why
    // the two deliberately disagree.
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\n[env]\nprovider = \"flox\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    let row = BorrowedStoreRow {
        store: store.clone(),
        fingerprint: "seam-fingerprint-not-a-real-providers".to_string(),
    };

    let outcome = up_with_provider(&manifest, &project_root, &UpOptions::default(), &row);

    let meta = devcroft::lifecycle::read_meta(&paths.meta);
    let _ = down(&sandbox_name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);

    assert_eq!(
        outcome.expect("up through the seam must succeed"),
        UpOutcome::Started
    );
    let meta = meta.unwrap().expect("up must have written meta.json");

    // Entry point 2: the fingerprint recorded is the row's, so `status`
    // will compare against what the row produced. A seam that left
    // fingerprinting on the name-dispatched path would have written flox's.
    assert_eq!(
        meta.env_fingerprint, "seam-fingerprint-not-a-real-providers",
        "the recorded fingerprint must come from the injected row, not from \
         the provider the manifest names"
    );

    // Entry point 1: the grants the row returned are the ones recorded, so
    // the environment `up` compiled against is the row's.
    assert!(
        meta.read_only_grants
            .iter()
            .any(|g| g == &store.to_string_lossy()),
        "the row's read-only grants must be the ones recorded; got {:?}",
        meta.read_only_grants
    );

    // **Entry point 3 is deliberately not asserted here, and the reason is a
    // finding rather than an omission.**
    //
    // An earlier version of this test did assert it, by re-compiling the
    // policy with `ROW_NAME` and checking the rendered output said
    // `provider:nix`. That assertion passed with the seam's third entry
    // point reverted to a manifest lookup — because the test supplied the
    // name itself. It measured its own argument.
    //
    // Reverting the mechanism and re-running is what caught it, and looking
    // for a non-vacuous version is what established why there isn't one:
    // `static_name` becomes an `Origin` on the compiled policy, `Meta` does
    // not record the provider name, and `CapabilityPlan` drops origins
    // before anything is persisted. So the value has **no effect observable
    // from outside `up`** today — `policy --render` and `why` recompute
    // from the manifest when the CLI runs them later.
    //
    // Threading it through the seam is still right: it is what stops an
    // injected row's grants being attributed to whatever the manifest
    // happens to say, the moment that *does* become observable (a fixture
    // row whose name differs from the manifest's is exactly that case, and
    // this test is already such a row). But it is congruence held by
    // construction, not by this assertion, and saying so beats shipping a
    // check that cannot fail.
}

//! The first neutral-surface test written against the row contract
//! (`test-runtime-fixture`), and the proof that the contract works.
//!
//! Everything asserted here is devcroft's own behaviour — a sandbox comes
//! up, reports healthy, notices its environment drifted, and tears down.
//! None of it is a claim about flox, nix or devbox, which is exactly why it
//! should not have to pick one. Today ~25 files in this suite do pick one,
//! purely to get past `up`.
//!
//! Run it against another row with `DEVCROFT_TEST_PROVIDER=flox`, or the
//! whole matrix with `=all`.

mod common;

use common::for_each_row;
use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down, status};

fn manifest_of(fx: &dyn common::ProviderFixture) -> devcroft::config::Manifest {
    let text = std::fs::read_to_string(fx.project_root().join("devcroft.toml")).unwrap();
    parse(&text).unwrap().0
}

/// `up` is idempotent, `status` reflects a live keeper, and `down` stops it.
///
/// The property is provider-independent, so the test is too. What differs
/// per row is only how the project got built, which is the fixture's job.
#[test]
fn up_reports_healthy_and_down_stops_it_on_every_row() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("lifecycle", |fx| {
        let manifest = manifest_of(fx);
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        let outcome = fx.bring_up(&UpOptions::default());
        assert_eq!(
            outcome.expect("up must succeed on row {}"),
            UpOutcome::Started,
            "row {}: first up must start a keeper",
            fx.name()
        );

        // Idempotent: a second `up` against a healthy sandbox is a no-op,
        // not a second keeper.
        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::AlreadyUp,
            "row {}: up must be idempotent",
            fx.name()
        );

        let st = status(&manifest).unwrap();
        assert!(
            format!("{:?}", st.keeper).contains("Healthy"),
            "row {}: status must report a healthy keeper, got {:?}",
            fx.name(),
            st.keeper
        );

        down(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
    });
}

/// A row that drifts its own environment definition is reported stale.
///
/// This is the test that most needs the fixture: the fingerprint is
/// computed from different files per provider — `manifest.toml` + lock,
/// `flake.nix` + `flake.lock`, `devbox.json` + lock — so a shared test
/// cannot know what to touch. `mutate_to_drift` is on the trait for exactly
/// this, and it is why the trait has a method that looks like a test
/// helper.
#[test]
fn a_drifted_environment_is_reported_stale_on_every_row() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("stale", |fx| {
        // Gated on the capability, never on the row's name. The injected row
        // cannot express this: `up` takes its provider through the seam but
        // `status` re-derives one from the manifest, so the row's fingerprint
        // is honoured going in and ignored coming out.
        if !fx.capabilities().staleness {
            eprintln!(
                "skipping stale on row {}: no staleness capability",
                fx.name()
            );
            return;
        }
        let manifest = manifest_of(fx);
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started
        );

        // `Option<bool>`: `None` means staleness could not be determined
        // (the provider could not be asked), which is neither fresh nor
        // stale and must not be read as either.
        let fresh = status(&manifest).unwrap();
        assert_eq!(
            fresh.env_stale,
            Some(false),
            "row {}: a freshly-upped sandbox must be reported fresh",
            fx.name()
        );

        fx.mutate_to_drift();

        let drifted = status(&manifest).unwrap();
        assert_eq!(
            drifted.env_stale,
            Some(true),
            "row {}: status must notice the environment definition changed",
            fx.name()
        );

        down(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
    });
}

/// The row's environment is real: its shell comes out of the closure, not
/// off the host.
///
/// `test-runtime-fixture` requires that no row satisfy the contract by
/// resolving its shell or toolchain from the host — a row that reached for
/// `/bin/sh` would be green precisely where devcroft's own regression once
/// was (`shell::resolve` picked `/usr/bin/dash` and every service died),
/// and the matrix would certify it.
///
/// Asserted here rather than inside each row so it holds for every row
/// automatically, including ones added later.
#[test]
fn every_row_resolves_its_shell_out_of_the_closure() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("shell", |fx| {
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);

        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started
        );
        let meta = devcroft::lifecycle::read_meta(&paths.meta)
            .unwrap()
            .expect("up records the shell it resolved");
        let shell = meta.shell.clone().expect("a row must resolve a shell");

        // **Inside one of the row's own declared grants, not under a
        // hardcoded `/nix/store`.**
        //
        // The store prefix was the obvious spelling and the wrong one: it is
        // a proxy for the property, which is "the shell is inside something
        // this sandbox is granted and can execute". `shell::resolve` was
        // generalized to exactly that in `add-test-runtime-fixture`, and a
        // store-prefix assertion here would reject a correct store-free row
        // — `add-nix-free-test-row` task 5.3.
        //
        // This is *stronger* than the old check, not weaker: it is compared
        // against what `up` actually recorded this sandbox as being granted,
        // so a shell resolved from anywhere the provider did not declare
        // fails it. A host shell has no grant containing it, which is the
        // regression this guards (`/usr/bin/dash`, every service dying with
        // `permission denied`).
        assert!(
            meta.read_only_grants
                .iter()
                .any(|g| shell.starts_with(g.as_str())),
            "row {}: the shell must be inside a path the row declared as a grant, \
             got {shell:?} with grants {:?} — a row backed by host tooling does not \
             satisfy this contract",
            fx.name(),
            meta.read_only_grants
        );

        down(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
    });
}

/// Neutral tests ask what a row *supports*, never what it is called.
///
/// A lint rather than a convention, because a convention only holds until
/// the first exception and this one is easy to breach by accident: a single
/// `if fx.name() == "flox"` reintroduces the per-provider branching the row
/// contract exists to remove, and makes every later one look normal.
/// `capabilities()` is the supported way to say "this row cannot do that".
#[test]
fn no_neutral_test_branches_on_a_rows_name() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        // Only files that actually use the row contract are in scope; the
        // provider-contract files name their provider on purpose.
        if !src.contains("mod common;") {
            continue;
        }
        for (n, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            // String literals are stripped before matching, so neither this
            // lint's own body nor an assert message that merely *prints*
            // `fx.name()` counts. Caught the honest way: the first version
            // flagged itself.
            let code = strip_string_literals(code);
            if code.contains(".name()") && (code.contains("==") || code.contains("match ")) {
                offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "neutral tests must gate on `capabilities()`, not on a row's name:\n{}",
        offenders.join("\n")
    );
}

/// Everything outside double-quoted literals on one line of Rust source.
///
/// Deliberately crude — it does not know about escapes or raw strings —
/// because its only job is keeping the name-branching lint above from
/// matching text that is being *quoted* rather than executed.
fn strip_string_literals(line: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    for c in line.chars() {
        match c {
            '"' => in_str = !in_str,
            _ if !in_str => out.push(c),
            _ => {}
        }
    }
    out
}

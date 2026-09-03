//! The provider-row fixture contract (`test-runtime-fixture`).
//!
//! A neutral-surface test — one that exercises devcroft's own behaviour
//! rather than a provider's — is written once against [`ProviderFixture`]
//! and runs against whichever row `DEVCROFT_TEST_PROVIDER` selects. The
//! point is that 25 of this suite's test files currently build a real flox
//! environment only to get past `up`, then assert something with no
//! provider content at all.
//!
//! **Why this lives in `tests/` and not in the crate.** It runs `flox
//! init`, `nix flake lock`, `devbox install`. That is test infrastructure,
//! and putting it behind the crate's `test-support` feature would carry
//! provider-invoking code in the library's own source for no product
//! reason. The seam the crate *does* expose (`up_with_provider`) is a
//! different thing and stays where it is.
//!
//! **Rows do not need the seam.** For flox, nix and devbox, the row writes
//! a `devcroft.toml` naming that provider and the test calls the ordinary
//! public `up`. Only a synthetic row would need `ProviderEntry` injection,
//! and that row does not exist yet — see this change's task 0, which
//! measured that `up` refuses an environment whose shell is not inside the
//! store. So the matrix over real providers works today with no feature
//! flag, which is the half worth having first.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// What a row can and cannot exercise.
///
/// **Consulted by tests; never their row's name.** A neutral test that
/// wrote `if fx.name() == "flox"` would reintroduce the per-provider
/// conditionals this contract exists to remove, and the first such branch
/// makes every later one look normal. Ask what the row *supports*.
#[derive(Debug, Clone, Copy)]
pub struct ProviderCapabilities {
    /// Supervised services (`[services]` → process-compose). Flox only:
    /// nix and devbox have no service concept, which is a property of
    /// those providers rather than of this fixture.
    pub services: bool,
    /// The provider runs a project-supplied activation hook that devcroft
    /// has to confine (`fix-provisioning-hooks`). Flox only.
    pub activation_hook: bool,
}

/// One row of the matrix: a project of a given provider's shape, set up on
/// this host and ready for `up`.
///
/// **`setup` is not on this trait**, unlike design.md's sketch, because an
/// associated function returning `Self` is not object-safe and the whole
/// point is handing tests a `&mut dyn ProviderFixture`. Construction is
/// [`fixture_for`]/[`for_each_row`] instead; the trait carries only what a
/// test asks of a row it already has.
pub trait ProviderFixture {
    /// The row's name, for reporting. Tests must not branch on it.
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> ProviderCapabilities;

    /// The project this row created, with a `devcroft.toml` already in it.
    fn project_root(&self) -> &Path;

    /// The sandbox name the row's `devcroft.toml` declares.
    fn sandbox_name(&self) -> &str;

    /// Change the project so its environment fingerprint changes.
    ///
    /// On the trait because a shared staleness test cannot know what to
    /// touch: the fingerprint comes from `manifest.toml` + lock for flox,
    /// `flake.nix` + `flake.lock` for nix, `devbox.json` + lock for devbox.
    fn mutate_to_drift(&mut self);
}

// ---------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------

/// The multi-system flake the nix row builds on.
///
/// Systems are enumerated rather than derived: `builtins.currentSystem` is
/// unavailable in pure flake evaluation, and a flake declaring only
/// `aarch64-linux` fails on an Apple Silicon host with "does not provide
/// attribute devShells.aarch64-darwin.default" — a fixture bug that reads
/// exactly like a provider regression. Same shape
/// `tests/nix_provider_e2e.rs` already proved out.
///
/// `bash` and `coreutils` are in it deliberately: `own-policy-baseline`
/// removed host toolchain access, so a bare `mkShell` leaves a sandbox with
/// no shell devcroft is allowed to run, and `up` refuses rather than
/// starting one that cannot work.
const NIX_FLAKE: &str = r#"
{
  description = "devcroft test-runtime-fixture nix row";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      shellFor = system:
        let pkgs = import nixpkgs { inherit system; };
        in pkgs.mkShell {
          packages = [ pkgs.bash pkgs.coreutils ];
          DEVCROFT_FIXTURE_ROW = "nix";
        };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
"#;

pub struct NixRow {
    root: PathBuf,
    sandbox: String,
}

impl ProviderFixture for NixRow {
    fn name(&self) -> &'static str {
        "nix"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            services: false,
            activation_hook: false,
        }
    }
    fn project_root(&self) -> &Path {
        &self.root
    }
    fn sandbox_name(&self) -> &str {
        &self.sandbox
    }
    fn mutate_to_drift(&mut self) {
        // The fingerprint is a content hash of `flake.nix` + `flake.lock`,
        // so any real edit to the flake moves it. A comment is enough and
        // avoids re-locking, which would need the network.
        let p = self.root.join("flake.nix");
        let mut s = std::fs::read_to_string(&p).unwrap();
        s.push_str("\n# drifted by the fixture\n");
        std::fs::write(&p, s).unwrap();
    }
}

pub struct FloxRow {
    root: PathBuf,
    sandbox: String,
}

impl ProviderFixture for FloxRow {
    fn name(&self) -> &'static str {
        "flox"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            services: true,
            activation_hook: true,
        }
    }
    fn project_root(&self) -> &Path {
        &self.root
    }
    fn sandbox_name(&self) -> &str {
        &self.sandbox
    }
    fn mutate_to_drift(&mut self) {
        let p = self.root.join(".flox/env/manifest.toml");
        let mut s = std::fs::read_to_string(&p).unwrap();
        s.push_str("\n# drifted by the fixture\n");
        std::fs::write(&p, s).unwrap();
    }
}

pub struct DevboxRow {
    root: PathBuf,
    sandbox: String,
}

impl ProviderFixture for DevboxRow {
    fn name(&self) -> &'static str {
        "devbox"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            services: false,
            activation_hook: false,
        }
    }
    fn project_root(&self) -> &Path {
        &self.root
    }
    fn sandbox_name(&self) -> &str {
        &self.sandbox
    }
    fn mutate_to_drift(&mut self) {
        let p = self.root.join("devbox.json");
        let mut s = std::fs::read_to_string(&p).unwrap();
        // devbox.json is JSONC, so a trailing comment is valid and does not
        // disturb the package set.
        s.push_str("\n// drifted by the fixture\n");
        std::fs::write(&p, s).unwrap();
    }
}

// ---------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------

/// A project root short enough for `sun_path` and already canonical.
///
/// Both halves are lessons this suite keeps relearning: the service
/// supervisor's socket lives at `<root>/.devcroft/<name>/services.sock` and
/// overflows 103 bytes under macOS's `$TMPDIR`, and `/tmp` on macOS is a
/// symlink whose un-canonicalized spelling a sandbox is refused
/// (docs/known-gaps.md).
fn fixture_root(row: &str, tag: &str) -> PathBuf {
    let root = PathBuf::from(format!("/tmp/dcfx-{row}-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn write_manifest(root: &Path, sandbox: &str, provider: &str) {
    std::fs::write(
        root.join("devcroft.toml"),
        format!("[sandbox]\nname = {sandbox:?}\n\n[env]\nprovider = {provider:?}\n"),
    )
    .unwrap();
}

/// Why a row could not be set up. Carried rather than discarded so the
/// matrix can name the reason instead of printing a bare "skip".
pub struct Unavailable(pub String);

fn setup_nix(tag: &str) -> Result<Box<dyn ProviderFixture>, Unavailable> {
    // Probes the *capability*, never the binary: `nix flake --help`
    // succeeds against an unreachable store, which is the mistake this
    // project has documented more than once.
    if !Command::new("nix")
        .args(["flake", "--help"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Err(Unavailable("a flakes-enabled nix is not on PATH".into()));
    }
    if !devcroft::provider::host_can_build_nix_closures() {
        return Err(Unavailable("this host cannot build nix closures".into()));
    }
    let root = fixture_root("nix", tag);
    std::fs::write(root.join("flake.nix"), NIX_FLAKE).unwrap();
    let lock = Command::new("nix")
        .arg("flake")
        .arg("lock")
        .arg(&root)
        .output()
        .map_err(|e| Unavailable(format!("nix flake lock could not run: {e}")))?;
    if !lock.status.success() {
        return Err(Unavailable(format!(
            "nix flake lock failed (likely no network for nixpkgs): {}",
            String::from_utf8_lossy(&lock.stderr).trim()
        )));
    }
    let sandbox = format!("fxnix{tag}{}", std::process::id());
    write_manifest(&root, &sandbox, "nix");
    Ok(Box::new(NixRow { root, sandbox }))
}

fn setup_flox(tag: &str) -> Result<Box<dyn ProviderFixture>, Unavailable> {
    if Command::new("flox").arg("--version").output().is_err()
        || !devcroft::provider::host_can_build_nix_closures()
    {
        return Err(Unavailable(
            "flox is not on PATH, or this host has no reachable nix store".into(),
        ));
    }
    let root = fixture_root("flox", tag);
    if !Command::new("flox")
        .arg("init")
        .current_dir(&root)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Err(Unavailable("flox init failed".into()));
    }
    // Same reason the nix row's flake names them: `own-policy-baseline`
    // removed host toolchain access, so a bare `flox init` leaves nothing
    // devcroft is allowed to run.
    let install = Command::new("flox")
        .args(["install", "bash", "coreutils"])
        .current_dir(&root)
        .output();
    match install {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Err(Unavailable(format!(
                "flox install bash coreutils failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )));
        }
        Err(e) => return Err(Unavailable(format!("flox install could not run: {e}"))),
    }
    let sandbox = format!("fxflox{tag}{}", std::process::id());
    write_manifest(&root, &sandbox, "flox");
    Ok(Box::new(FloxRow { root, sandbox }))
}

fn setup_devbox(tag: &str) -> Result<Box<dyn ProviderFixture>, Unavailable> {
    if !Command::new("devbox")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
        || !devcroft::provider::host_can_build_nix_closures()
    {
        return Err(Unavailable(
            "devbox is not on PATH, or this host has no reachable nix store".into(),
        ));
    }
    let root = fixture_root("devbox", tag);
    if !Command::new("devbox")
        .arg("init")
        .current_dir(&root)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Err(Unavailable("devbox init failed".into()));
    }
    // `devbox install` must run here, host-side: devcroft refuses to
    // provision if resolving would rewrite the lockfile, so a row that
    // handed `up` an unresolved project would fail at layer `provider` for
    // a reason that has nothing to do with the test.
    for pkg in ["bash", "coreutils"] {
        let out = Command::new("devbox")
            .args(["add", pkg])
            .current_dir(&root)
            .output();
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return Err(Unavailable(format!(
                    "devbox add {pkg} failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )));
            }
            Err(e) => return Err(Unavailable(format!("devbox add could not run: {e}"))),
        }
    }
    // `add` records the package; `install` is what materializes and settles
    // the lockfile. Without this, devcroft refuses at layer `provider` --
    // "devbox resolved packages while capturing the environment and rewrote
    // devbox.lock" -- because provisioning must not resolve versions. Found
    // by running the matrix, not by reading: the `add`-only row got all the
    // way to `up` before failing.
    match Command::new("devbox")
        .arg("install")
        .current_dir(&root)
        .output()
    {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Err(Unavailable(format!(
                "devbox install failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )));
        }
        Err(e) => return Err(Unavailable(format!("devbox install could not run: {e}"))),
    }
    let sandbox = format!("fxdbx{tag}{}", std::process::id());
    write_manifest(&root, &sandbox, "devbox");
    Ok(Box::new(DevboxRow { root, sandbox }))
}

/// How a row is built: given a tag, either a ready fixture or the reason
/// this host cannot provide one.
type SetupFn = fn(&str) -> Result<Box<dyn ProviderFixture>, Unavailable>;

/// Every row this contract knows, in matrix order.
const ROWS: &[(&str, SetupFn)] = &[
    ("nix", setup_nix),
    ("flox", setup_flox),
    ("devbox", setup_devbox),
];

/// The default row when `DEVCROFT_TEST_PROVIDER` is unset.
///
/// Nix flakes, not a synthetic row: local `cargo test` should stay
/// realistic — a real closure, a shell resolved out of it, a real loader —
/// and the cheap row has to be asked for by name. A developer running the
/// suite is entitled to assume they ran the realistic one.
const DEFAULT_ROW: &str = "nix";

/// Run `body` against every row this selection covers, reporting each
/// row's outcome.
///
/// Selection: `DEVCROFT_TEST_PROVIDER` unset → the default row;
/// `nix|flox|devbox` → that row; `all` → every row, skipping the ones this
/// host cannot set up.
///
/// **No fallback, ever.** A row that cannot be set up is reported as
/// skipped, by name and with the reason — it is never silently replaced by
/// a cheaper one.
///
/// **What a skip then does to the run depends on whether anything ran at
/// all**, and this is the rule that took a measurement to settle:
///
/// - default row unavailable → **fail**, with the remedy. Nobody asked for
///   it explicitly, so a developer who runs `cargo test` and sees green
///   must not have been quietly moved to something weaker.
/// - one explicitly-selected row, unavailable → **fail**. The report says
///   "skip", but the run does not pass: someone asked for that row
///   specifically and it did not run, so a green board would mean exactly
///   "nothing was tested". A CI job named `integration-devbox` that goes
///   green on a runner with no devbox is the trap this whole contract
///   exists to close.
/// - `=all`, some rows unavailable → those rows skip and the run passes on
///   the rest. Only if *every* row skipped does it fail.
///
/// The first draft of this change's task 6.2 said the opposite for the
/// middle case ("only an unavailable one skips"). Running it is what showed
/// that to be the wrong half of the trade; the task is corrected rather
/// than the behaviour.
pub fn for_each_row(tag: &str, body: impl Fn(&mut dyn ProviderFixture)) {
    let selection = std::env::var("DEVCROFT_TEST_PROVIDER").ok();
    let explicit = selection.is_some();
    let selection = selection.unwrap_or_else(|| DEFAULT_ROW.to_string());

    let wanted: Vec<_> = if selection == "all" {
        ROWS.iter().collect()
    } else {
        match ROWS.iter().find(|(n, _)| *n == selection) {
            Some(row) => vec![row],
            None => panic!(
                "DEVCROFT_TEST_PROVIDER={selection:?} names no row; known rows: \
                 {}, or `all`",
                ROWS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            ),
        }
    };

    let mut report: Vec<String> = Vec::new();
    let mut ran = 0usize;
    let mut failed = 0usize;
    for (name, setup) in wanted {
        match setup(tag) {
            Ok(mut fx) => {
                // Caught so a failing row still leaves a matrix behind. The
                // first version let the panic escape, and the whole report
                // went with it: `=all` printed nothing at all, so a run
                // covering three rows told you about one. The run still
                // fails below -- an available-but-broken row is a failure,
                // never a skip -- it just fails legibly.
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(fx.as_mut())));
                let _ = std::fs::remove_dir_all(fx.project_root());
                match outcome {
                    Ok(()) => {
                        report.push(format!("{name} ok"));
                        ran += 1;
                    }
                    Err(_) => {
                        report.push(format!("{name} FAILED"));
                        failed += 1;
                    }
                }
            }
            Err(Unavailable(reason)) => {
                // The default row is the one case where unavailability is a
                // failure rather than a skip -- see this function's doc.
                if !explicit {
                    panic!(
                        "the default test row ({name}) could not be set up: {reason}\n\
                         devcroft's suite defaults to a real Nix environment on purpose. \
                         It will not fall back to something cheaper behind your back; \
                         select another row explicitly with \
                         DEVCROFT_TEST_PROVIDER=flox|devbox|all."
                    );
                }
                report.push(format!("{name} skip({reason})"));
            }
        }
    }

    eprintln!("provider matrix [{tag}]: {}", report.join(", "));

    // An available row that failed fails the run. Reported after the matrix
    // rather than at the moment of failure so `=all` still tells you about
    // the rows that came after it.
    assert_eq!(failed, 0, "row(s) failed: {}", report.join(", "));

    // A run in which every row skipped is not a pass. Without this the
    // matrix would be greenest exactly when it tested nothing -- the
    // failure mode this whole contract exists to remove.
    assert!(
        ran > 0,
        "every selected row skipped, so this test asserted nothing: {}",
        report.join(", ")
    );
}

/// Single-row convenience for tests that cannot be written as a closure.
///
/// Same selection and same no-fallback rule as [`for_each_row`], but it
/// returns `None` on an unavailable explicitly-selected row so the caller
/// can skip. Prefer `for_each_row`, which cannot forget to report.
pub fn fixture_for(tag: &str) -> Option<Box<dyn ProviderFixture>> {
    let selection = std::env::var("DEVCROFT_TEST_PROVIDER").ok();
    let explicit = selection.is_some();
    let selection = selection.unwrap_or_else(|| DEFAULT_ROW.to_string());
    let name = if selection == "all" {
        DEFAULT_ROW
    } else {
        &selection
    };
    let (name, setup) = ROWS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("DEVCROFT_TEST_PROVIDER={selection:?} names no row"));
    match setup(tag) {
        Ok(fx) => Some(fx),
        Err(Unavailable(reason)) => {
            assert!(
                explicit,
                "the default test row ({name}) could not be set up: {reason}\n\
                 select another row explicitly with DEVCROFT_TEST_PROVIDER=flox|devbox."
            );
            eprintln!("skipping: row {name} unavailable: {reason}");
            None
        }
    }
}

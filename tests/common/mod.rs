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
    /// `status` can tell whether this row's environment drifted.
    ///
    /// **False for the injected row, and that is a seam limitation rather
    /// than a property of the row** — worth stating precisely because it is
    /// the kind of gap a capability flag can quietly bury. `up` takes its
    /// provider through the injection seam, but `status` re-derives one from
    /// `manifest.env.provider` (`lifecycle::status` → `provider::is_stale`),
    /// exactly as `policy --render` re-derives rule origins. So a row with
    /// no provider to name gets its fingerprint honoured on the way in and
    /// ignored on the way out.
    ///
    /// Closing it means giving `status` an injection point too. Until then a
    /// neutral staleness test gates on this rather than skipping by name.
    pub staleness: bool,
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

    /// Bring this row's sandbox up.
    ///
    /// **On the trait because rows do not all reach `up` the same way**, and
    /// pretending they do would push a `cfg` into every neutral test. A real
    /// provider row names its provider in `devcroft.toml` and goes through
    /// the ordinary public `up`; a row with no provider at all has to go
    /// through the injection seam. Which of those a row is stays the row's
    /// business, exactly like `setup` and `mutate_to_drift`.
    fn bring_up(&self, opts: &devcroft::lifecycle::UpOptions) -> UpResult {
        let manifest = self.manifest();
        devcroft::lifecycle::up(&manifest, self.project_root(), opts)
    }

    /// The manifest this row wrote, parsed.
    fn manifest(&self) -> devcroft::config::Manifest {
        let text = std::fs::read_to_string(self.project_root().join("devcroft.toml")).unwrap();
        devcroft::config::parse(&text).unwrap().0
    }
}

type UpResult = Result<devcroft::lifecycle::UpOutcome, devcroft::lifecycle::UpError>;

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
            staleness: true,
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
            staleness: true,
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
            staleness: true,
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

// ---------------------------------------------------------------------
// The Nix-free row (`add-nix-free-test-row`)
//
// Behind `test-support` because it drives `up` through the injection seam,
// which only exists under that feature. A default `cargo test` therefore
// does not have this row at all -- which is also the strongest possible
// form of "it is not the default".
// ---------------------------------------------------------------------

/// Where this row finds its shell.
///
/// **Provisioning the binary is deliberately not this row's job yet.** How
/// it gets there -- fetched and hash-pinned, vendored, or built from source
/// -- is an open decision in `add-nix-free-test-row` with supply-chain and
/// licence consequences, and picking one silently here would settle it by
/// accident. So the row consumes a shell someone else put in place and
/// reports itself unavailable, with instructions, when nobody has.
///
/// Measured on macOS 15.7.4: a `dash` built from source (6s with the Xcode
/// command-line tools) works end to end -- `up` reaches `Started`, the
/// shell resolves inside the row's own grant, and a real session returns
/// its output. That is what this row is waiting for a supply story for, not
/// a hypothesis.
#[cfg(feature = "test-support")]
const ROW_SHELL_ENV: &str = "DEVCROFT_TEST_ROW_SHELL";

/// The shell this row uses by default: devcroft's own `examples/test-row-sh`,
/// a one-line wrapper around the `brush` crate.
///
/// Located relative to the running test binary rather than built here, so a
/// fixture never shells out to cargo mid-suite. `cargo build --example
/// test-row-sh` puts it there; the row reports itself unavailable, with that
/// command, when it is missing.
fn bundled_row_shell() -> Option<PathBuf> {
    // .../target/<profile>/deps/<test binary>  ->  .../target/<profile>/examples/
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.parent()?.join("examples").join("test-row-sh");
    candidate.is_file().then_some(candidate)
}

#[cfg(feature = "test-support")]
pub struct NixFreeRow {
    root: PathBuf,
    sandbox: String,
    dir: PathBuf,
}

#[cfg(feature = "test-support")]
impl ProviderFixture for NixFreeRow {
    fn name(&self) -> &'static str {
        "test"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        // No `process-compose` shipped, so no services. Deciding whether to
        // ship one is `add-nix-free-test-row` task 4.1; until then a neutral
        // services test skips on this row by capability, never by name.
        ProviderCapabilities {
            services: false,
            activation_hook: false,
            // See the field's own doc: `status` re-derives its provider from
            // the manifest, so this row's fingerprint is honoured by `up`
            // and invisible to `status`.
            staleness: false,
        }
    }
    fn project_root(&self) -> &Path {
        &self.root
    }
    fn sandbox_name(&self) -> &str {
        &self.sandbox
    }
    fn mutate_to_drift(&mut self) {
        // This row's "environment definition" is the directory it owns, and
        // its fingerprint is supplied by the fixture rather than read off a
        // provider manifest -- so drifting means changing what the row will
        // report, which the marker file below stands in for.
        let p = self.dir.join("fingerprint");
        let current = std::fs::read_to_string(&p).unwrap_or_default();
        std::fs::write(&p, format!("{current}drift\n")).unwrap();
    }

    /// Through the seam, because this row has no provider to name.
    fn bring_up(&self, opts: &devcroft::lifecycle::UpOptions) -> UpResult {
        let manifest = self.manifest();
        devcroft::test_support::up_with_provider(&manifest, &self.root, opts, self)
    }
}

/// The row is its own `ProviderEntry`: it resolves to the directory it
/// built, fingerprints from the marker `mutate_to_drift` edits, and reports
/// a **real provider's name** -- `static_name` becomes `Origin::Provider` in
/// `policy --render`, and `provider-injection-seam` requires no origin token
/// exist that a real provider could not emit.
#[cfg(feature = "test-support")]
impl devcroft::provider::ProviderEntry for NixFreeRow {
    fn resolve(
        &self,
        _project_root: &Path,
    ) -> Result<devcroft::provider::Resolution, devcroft::provider::ProviderError> {
        let mut env = std::collections::BTreeMap::new();
        env.insert(
            "PATH".to_string(),
            self.dir.join("bin").to_string_lossy().into_owned(),
        );
        Ok(devcroft::provider::Resolution {
            env,
            unset: Vec::new(),
            read_only_grants: vec![self.dir.to_string_lossy().into_owned()],
            activation_script: None,
            services: devcroft::provider::ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }

    fn fingerprint(
        &self,
        _project_root: &Path,
    ) -> Result<String, devcroft::provider::ProviderError> {
        Ok(std::fs::read_to_string(self.dir.join("fingerprint")).unwrap_or_default())
    }

    fn static_name(&self) -> &'static str {
        "nix"
    }
}

/// Runs `shell -c "exit 0"` and waits at most `timeout` for it.
///
/// **The bounded wait is the point, not defensiveness.** A copied macOS
/// platform binary does not fail when executed -- it hangs, measured -- and
/// a fixture that blocks forever on setup reads as a slow test rather than
/// a broken row. So the row proves its shell answers before handing it to
/// `up`, and a shell that does not answer makes the row unavailable.
///
/// **It does not reap the process it gave up on, and that is deliberate.**
/// Measured: a copied macOS platform binary lands in state `UE` --
/// uninterruptible kernel wait, exiting -- where it survives `SIGKILL`.
/// `kill()` then `wait()` therefore blocks forever, which is how the first
/// version of this function turned a five-second bound into a ten-minute
/// hang. The signal is still sent, on the chance the process is merely
/// slow; nothing waits for the answer.
///
/// The consequence worth knowing: probing a bad candidate can leak an
/// unkillable process for the life of the machine. So this is a last line
/// of defence, not a licence to point the row at a copied system binary --
/// there is no userspace way to clean that up afterwards.
#[cfg(feature = "test-support")]
fn shell_answers(shell: &Path, timeout: std::time::Duration) -> bool {
    let Ok(mut child) = Command::new(shell)
        .args(["-c", "exit 0"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    // Signalled but deliberately not reaped -- see the doc.
                    let _ = child.kill();
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => return false,
        }
    }
}

#[cfg(feature = "test-support")]
fn setup_test_row(tag: &str) -> Result<Box<dyn ProviderFixture>, Unavailable> {
    // The bundled shell by default; the env var stays as an override for
    // trying a different one without editing code.
    let shell = match std::env::var(ROW_SHELL_ENV) {
        Ok(p) => PathBuf::from(p),
        Err(_) => match bundled_row_shell() {
            Some(p) => p,
            None => {
                return Err(Unavailable(
                    "the row's shell is not built; run `cargo build --example \
                     test-row-sh` (or set DEVCROFT_TEST_ROW_SHELL to another \
                     POSIX shell that is not from the nix store or the host)"
                        .to_string(),
                ));
            }
        },
    };
    if !shell.is_file() {
        return Err(Unavailable(format!(
            "{ROW_SHELL_ENV}={} is not a file",
            shell.display()
        )));
    }
    if !shell_answers(&shell, std::time::Duration::from_secs(5)) {
        return Err(Unavailable(format!(
            "{} did not answer `-c 'exit 0'` within 5s; a copied macOS platform \
             binary hangs exactly like this",
            shell.display()
        )));
    }

    let dir = fixture_root("testrow-env", tag);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(&shell, bin.join("sh"))
        .map_err(|e| Unavailable(format!("could not place the row's shell: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(bin.join("sh"), std::fs::Permissions::from_mode(0o755));
    }
    std::fs::write(dir.join("fingerprint"), "base\n").unwrap();

    let root = fixture_root("testrow", tag);
    let sandbox = format!("fxtest{tag}{}", std::process::id());
    // No `[env] provider` line: this row drives `up` through the seam, so
    // the manifest's provider is never consulted. It still needs a manifest
    // for the sandbox name.
    std::fs::write(
        root.join("devcroft.toml"),
        format!("[sandbox]\nname = {sandbox:?}\n"),
    )
    .unwrap();

    Ok(Box::new(NixFreeRow { root, sandbox, dir }))
}

/// How a row is built: given a tag, either a ready fixture or the reason
/// this host cannot provide one.
type SetupFn = fn(&str) -> Result<Box<dyn ProviderFixture>, Unavailable>;

/// Every row this contract knows, in matrix order.
const ROWS: &[(&str, SetupFn)] = &[
    ("nix", setup_nix),
    ("flox", setup_flox),
    ("devbox", setup_devbox),
    // Only under `test-support`, because it drives `up` through the
    // injection seam. A default `cargo test` does not have this row at all,
    // which is the strongest available form of "it is not the default".
    #[cfg(feature = "test-support")]
    ("test", setup_test_row),
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

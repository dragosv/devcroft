//! The `swift` provider (add-swift-provider): resolves a SwiftPM project's
//! build environment host-side, before any sandbox restriction applies.
//!
//! **This is devcroft's first artifact-tier provider, and the first one
//! that fails the qualification test in `docs/decisions.md` §1.** Both are
//! deliberate, both are the project owner's decision over a recorded
//! objection, and neither is hidden from the user — see
//! [`SwiftProvider::resolve`]'s disclosure and `docs/known-gaps.md`.
//!
//! The module is organized around one distinction that the rest of it
//! depends on: **where the toolchain is** is answered with data, and
//! **whether this package needs a lockfile** cannot be. Conflating the two
//! would have made every resolution run project code for both answers
//! instead of one.

use super::capture;
use super::{Provider, ProviderError, Resolution};
use crate::paths::resolve_on_path;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct SwiftProvider;

impl Provider for SwiftProvider {
    /// Resolve a Swift package's build environment.
    ///
    /// Preconditions run in a fixed order (the same rule `nix.rs` states)
    /// so a failure names the most specific fix available rather than
    /// surfacing SwiftPM's own, less specific error first: no
    /// `Package.swift` is a different problem from no toolchain, which is
    /// a different problem from an unresolved dependency graph.
    ///
    /// **This function executes the project's own code**, and says so in
    /// its result. `Package.swift` is a Swift program: SwiftPM compiles it
    /// with `swiftc` and runs the resulting binary to obtain the package
    /// description. Measured on Swift 6.1.2 — a deliberately invalid
    /// manifest reports `Invalid manifest (compiled with: [".../swiftc",
    /// …, "Package.swift", "-o", "…/probe-manifest"])`. No entry point
    /// returns the package graph without doing this: `dump-package`,
    /// `resolve`, `describe` and `build` all evaluate it, and there is no
    /// counterpart to nix's `print-dev-env --json` or devbox's
    /// `shellenv --pure`.
    ///
    /// SwiftPM sandboxes that evaluation on macOS, which is worth less
    /// than it sounds: it blocks **writes** and not **reads**. Manifest
    /// code exfiltrates host state through the package data itself —
    /// measured, with the sandbox on, `dump-package` returning
    /// `"name": "LEAK[HOME=/Users/…][SSH=id_ed25519,known_hosts,…]"`.
    /// On Linux there is no Seatbelt and no manifest sandbox at all.
    fn resolve(&self, project_root: &Path) -> Result<Resolution, ProviderError> {
        ensure_package_present(project_root)?;
        let swift_bin = resolve_on_path("swift").ok_or(ProviderError::MissingBinary {
            provider: "swift",
            hint: "devcroft doctor",
        })?;

        // Data only: no project file is evaluated to answer this.
        let toolchain = probe_toolchain(&swift_bin)?;

        // The one call that runs project code, and the only thing read
        // out of it is whether the package declares dependencies.
        let declares_dependencies = package_declares_dependencies(&swift_bin, project_root)?;
        ensure_lock_present(project_root, declares_dependencies)?;

        let baseline = capture::canonical_base_env()?;
        let tmpdir = project_tmpdir(project_root)?;
        let activated = activated_env(&baseline, &swift_bin, &toolchain, Some(&tmpdir));

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: toolchain.grants(&swift_bin),
            // Swift has no service concept. Declared rather than left
            // empty for the same reason `nix.rs` states: `up` must be
            // able to tell "cannot do services" from "can, none
            // declared", so a project asking for them fails loudly.
            services: super::ServiceSupport::Unsupported,
            // **Unconditional, and that is the design.** The provider
            // cannot inspect `Package.swift` to decide whether this
            // particular one was dangerous — deciding that would mean
            // running it, which is the thing being disclosed. A warning
            // that is sometimes suppressed teaches people to expect
            // silence.
            ran_activation_hook: true,
            // Nothing is deferred into the sandbox: unlike flox's
            // `[hook].on-activate`, the code here is not a separable
            // hook that could be run later under policy. It *is* the
            // manifest, it has already run, and pretending otherwise by
            // handing back a script would misrepresent what happened.
            activation_script: None,
        })
    }
}

/// Where the host's Swift toolchain lives, read from
/// `swift -print-target-info` — JSON, and no project file is evaluated to
/// produce it.
///
/// Every field here becomes a `provider:swift` grant, which is why they
/// are *measured* rather than hardcoded: the artifact tier's whole
/// premise is that the runtime links against host libraries, so the paths
/// differ between a Command Line Tools install, a full Xcode, a
/// Swift.org tarball, and a distro package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Toolchain {
    /// Directories holding the Swift runtime libraries the built binary
    /// links against.
    runtime_library_paths: Vec<String>,
    /// The toolchain's resource directory (module maps, stdlib
    /// interfaces) — `swiftc` cannot compile without it.
    runtime_resource_path: String,
    /// The platform SDK, on hosts that have one. `None` on Linux, where
    /// there is no separate SDK to grant.
    sdk_path: Option<String>,
    /// The selected developer directory, on macOS — see
    /// [`probe_developer_dir`] for why this travels as data rather than
    /// as a grant.
    developer_dir: Option<String>,
}

impl Toolchain {
    /// The read-only paths the compiled policy must grant, deduplicated
    /// and shortest-first so a parent that already covers a child is not
    /// followed by the redundant child.
    ///
    /// Includes the directory of the resolved `swift` binary itself: on
    /// macOS that is `/usr/bin` (a shim) and the real toolchain lives
    /// elsewhere, so granting only the library paths would produce a
    /// sandbox that cannot invoke the compiler it can read.
    pub(super) fn grants(&self, swift_bin: &Path) -> Vec<String> {
        let mut out: Vec<String> = self.runtime_library_paths.clone();
        out.push(self.runtime_resource_path.clone());
        if let Some(sdk) = &self.sdk_path {
            out.push(sdk.clone());
        }
        // The developer directory is granted as well as injected: the
        // variable tells the shim where to look, and the sandbox still
        // has to be allowed to read what it finds there.
        if let Some(dev) = &self.developer_dir {
            out.push(dev.clone());
        }
        if let Some(dir) = swift_bin.parent() {
            out.push(dir.to_string_lossy().into_owned());
        }
        if let Some(dir) = host_shell_dir() {
            out.push(dir);
        }
        out.sort();
        out.dedup();
        out.retain(|p| !p.is_empty());
        out
    }
}

/// `up` fails at layer `provider` with the `swift package init` hint
/// (spec: "Missing environment, not missing feature") rather than letting
/// SwiftPM produce its own, less specific error.
fn ensure_package_present(project_root: &Path) -> Result<(), ProviderError> {
    if project_root.join("Package.swift").is_file() {
        Ok(())
    } else {
        Err(ProviderError::NoEnvironment {
            provider: "swift",
            hint: "swift package init",
        })
    }
}

/// A package with dependencies but no `Package.resolved` has nothing
/// pinning them — resolving it would mean the same manifest can produce a
/// different dependency set depending on when `up` ran.
///
/// **Conditional on purpose.** SwiftPM does not create `Package.resolved`
/// for a package with no external dependencies, so requiring it
/// unconditionally would refuse valid projects — a rejection with no
/// remedy, which `docs/decisions.md` calls out as worse than a missing
/// feature.
fn ensure_lock_present(
    project_root: &Path,
    declares_dependencies: bool,
) -> Result<(), ProviderError> {
    if !declares_dependencies || project_root.join("Package.resolved").is_file() {
        return Ok(());
    }
    Err(ProviderError::MissingLock {
        provider: "swift",
        hint: "swift package resolve",
    })
}

/// Ask SwiftPM for the package description and read one thing out of it:
/// whether any dependency is declared.
///
/// **This is the criterion-4 violation, confined to one call.** It is
/// kept because the alternative is worse: skipping it would leave
/// `Package.resolved` unenforced, dropping criterion 2 (restorable
/// lockfile) as well as 4, and leaving the provider with no
/// reproducibility claim at all. If project code is going to run, it
/// should at least buy the lockfile guarantee.
///
/// `--disable-sandbox` is deliberately **not** passed. SwiftPM's own
/// manifest sandbox blocks writes on macOS, which is a partial mitigation
/// devcroft has no reason to switch off, and passing the flag would make
/// devcroft responsible for a weakening it did not need.
fn package_declares_dependencies(
    swift_bin: &Path,
    project_root: &Path,
) -> Result<bool, ProviderError> {
    let output = Command::new(swift_bin)
        .arg("package")
        .arg("dump-package")
        .current_dir(project_root)
        .output()
        .map_err(|e| {
            ProviderError::ResolutionFailed(format!("running `swift package dump-package`: {e}"))
        })?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`swift package dump-package` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    parse_declares_dependencies(&output.stdout)
}

/// The one field consumed from `dump-package`'s output.
///
/// Deliberately narrow. The full package description carries targets,
/// products, platforms and more, and every one of them is a thing this
/// provider would then have to keep working across SwiftPM schema
/// versions. Reading only `dependencies` keeps the blast radius of a
/// schema change to the single decision it feeds.
pub(super) fn parse_declares_dependencies(raw: &[u8]) -> Result<bool, ProviderError> {
    let parsed: serde_json::Value = serde_json::from_slice(raw).map_err(|e| {
        ProviderError::ResolutionFailed(format!("parsing `swift package dump-package` output: {e}"))
    })?;
    match parsed.get("dependencies") {
        Some(serde_json::Value::Array(deps)) => Ok(!deps.is_empty()),
        // A description with no `dependencies` key at all is a schema
        // devcroft does not recognize. Treated as "no dependencies"
        // would silently drop the lockfile requirement, so it fails
        // instead — the same rule `nix.rs` follows for an unparseable
        // `print-dev-env`.
        _ => Err(ProviderError::ResolutionFailed(
            "`swift package dump-package` output has no `dependencies` array; \
             this SwiftPM version is not one devcroft can read"
                .to_string(),
        )),
    }
}

/// Read the toolchain's own paths from `swift -print-target-info`.
///
/// Runs with a fixed environment for the same reason
/// `capture::canonical_base_env` exists: the answer must depend on the
/// host's toolchain, not on the shell that happened to run `up`.
fn probe_toolchain(swift_bin: &Path) -> Result<Toolchain, ProviderError> {
    let base = capture::canonical_base_env()?;
    let output = Command::new(swift_bin)
        .arg("-print-target-info")
        .env_clear()
        .envs(&base)
        .output()
        .map_err(|e| {
            ProviderError::ResolutionFailed(format!("running `swift -print-target-info`: {e}"))
        })?;

    if !output.status.success() {
        return Err(ProviderError::ResolutionFailed(format!(
            "`swift -print-target-info` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let mut toolchain = parse_target_info(&output.stdout)?;
    toolchain.sdk_path = probe_sdk_path();
    toolchain.developer_dir = probe_developer_dir();
    Ok(toolchain)
}

/// Parse the `paths` block of `swift -print-target-info`.
///
/// Measured shape (Swift 6.1.2, macOS):
///
/// ```json
/// "paths": {
///   "runtimeLibraryPaths": ["/Library/Developer/CommandLineTools/usr/lib/swift/macosx",
///                           "/usr/lib/swift"],
///   "runtimeResourcePath": "/Library/Developer/CommandLineTools/usr/lib/swift"
/// }
/// ```
pub(super) fn parse_target_info(raw: &[u8]) -> Result<Toolchain, ProviderError> {
    let parsed: serde_json::Value = serde_json::from_slice(raw).map_err(|e| {
        ProviderError::ResolutionFailed(format!("parsing `swift -print-target-info` output: {e}"))
    })?;

    let paths = parsed.get("paths").ok_or_else(|| {
        ProviderError::ResolutionFailed(
            "`swift -print-target-info` output has no `paths` block; this Swift \
             version is not one devcroft can read"
                .to_string(),
        )
    })?;

    let runtime_resource_path = paths
        .get("runtimeResourcePath")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ProviderError::ResolutionFailed(
                "`swift -print-target-info` reported no `runtimeResourcePath`; without \
                 it the sandbox cannot be granted the toolchain's resource directory"
                    .to_string(),
            )
        })?
        .to_string();

    let runtime_library_paths = paths
        .get("runtimeLibraryPaths")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    Ok(Toolchain {
        runtime_library_paths,
        runtime_resource_path,
        sdk_path: None,
        developer_dir: None,
    })
}

/// The developer directory macOS's toolchain shims resolve before doing
/// anything, discovered host-side and injected as `DEVELOPER_DIR`.
///
/// **This replaced a grant, and the reason is worth keeping.**
/// `/usr/bin/swift` is a shim: it asks `xcode-select` where the real
/// toolchain is, and `xcode-select` reads `/var/select/developer_dir`.
/// Inside the sandbox that read failed and the build died with
///
/// ```text
/// xcode-select: error: unable to read data link at '/var/select/developer_dir',
///   expected symbolic link (Operation not permitted)
/// xcode-select: note: No developer tools were found, requesting install.
/// ```
///
/// — a message naming neither devcroft nor a policy, telling the user to
/// install an Xcode they already have.
///
/// The obvious fix was to grant `/var/select`. **It does not work, and
/// measuring why is what produced this function.** `/var` is a symlink to
/// `/private/var` on macOS, and a grant does not cover the symlinked
/// spelling of its own path there — a shipped, published defect
/// (`docs/known-gaps.md`). Granting *both* spellings does not help
/// either: `/private/var/select` becomes readable and `/var/select` stays
/// denied, which is the spelling the tool actually opens. The same is
/// true of devcroft's own baseline — `/var/db/dyld` is granted in both
/// spellings and `ls /var/db/dyld` is refused inside. So the grant route
/// renders correctly in `policy --render` and enforces nothing, which is
/// precisely the failure shape that gap warns about.
///
/// Injecting the answer is strictly better than working around the
/// defect. `DEVELOPER_DIR` is the documented override that makes
/// `xcode-select` skip the lookup entirely, so nothing inside the sandbox
/// needs to resolve a host symlink at all — the same shape as `SDKROOT`
/// below, and the same shape as every other provider here: resolve
/// host-side at `up`, inject the result, let the sandbox need no host
/// lookup of its own.
///
/// `None` off macOS, and on a macOS host with no selected developer
/// directory — where a Swift toolchain that needs one would not work on
/// the host either.
fn probe_developer_dir() -> Option<String> {
    let xcode_select = resolve_on_path("xcode-select")?;
    let output = Command::new(xcode_select).arg("-p").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// The directory holding the host's POSIX shell, declared as a
/// `provider:swift` grant.
///
/// **This is the artifact tier's defining cost, made visible in the
/// compiled policy — and this provider is the first case where it is
/// load-bearing rather than theoretical.**
///
/// `src/shell.rs` resolves the shell devcroft itself needs (SSH login
/// sessions, `devcroft shell`'s fallback, the command process-compose
/// runs each service through) and accepts a candidate *only* when it
/// falls inside a path the provider declared. Its doc comment states the
/// consequence precisely: "A host shell is still refused, because no
/// provider declares a grant containing one." For the three closure
/// providers that is right — their shell comes from the store.
///
/// A Swift toolchain is not a closure and ships no shell, so without this
/// grant `up` fails with "no POSIX shell found in this environment or its
/// closure" — measured, on the first real run of this provider. The fix
/// is not to weaken the shell guard, which protects correctness (its
/// recorded failure was picking `/usr/bin/dash` and every service dying
/// with `permission denied`). It is for the host-linked provider to
/// declare the host path it depends on, exactly as
/// `Resolution::read_only_grants` requires of the artifact tier, so the
/// grant appears in `policy --render` with a `provider:swift` origin
/// instead of being inherited invisibly.
///
/// `add-test-runtime-fixture` anticipated this: it generalized the shell
/// guard from a literal `/nix/store` to "inside a declared grant"
/// specifically for "a provider whose environment is not store-backed",
/// and noted `ResolvedShell::grant` had been an `Option` for it all
/// along. `swift` is that provider arriving.
///
/// Discovered rather than hardcoded to `/bin`, so a host that keeps its
/// shell elsewhere still works and the grant still names a real path.
fn host_shell_dir() -> Option<String> {
    let sh = resolve_on_path(crate::shell::SHELL_NAME)?;
    let real = sh.canonicalize().unwrap_or(sh);
    Some(real.parent()?.to_string_lossy().into_owned())
}

/// The platform SDK, on hosts that have one.
///
/// Best-effort by design: `xcrun` exists only on macOS, and a Linux
/// toolchain has no separate SDK to grant, so its absence is a normal
/// outcome rather than an error. A failure here must not fail `up` — the
/// grant set is simply smaller, and a build that then cannot find the SDK
/// fails loudly with the policy visible in `policy --render`.
fn probe_sdk_path() -> Option<String> {
    let xcrun = resolve_on_path("xcrun")?;
    let output = Command::new(xcrun).arg("--show-sdk-path").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// The environment a Swift build needs inside the sandbox, expressed as
/// the post-activation map `capture::changed_env` diffs against the
/// baseline.
///
/// Two entries, and no more than is justified:
///
/// - `PATH` gains the toolchain's own `bin`, so `swift` resolves to the
///   same binary `up` probed rather than to whatever the sandbox's
///   `PATH` would otherwise find.
/// - `SDKROOT`, where a platform SDK exists, so `swiftc` finds it without
///   consulting `xcrun` — which is a host tool the sandbox is not granted.
/// - `DEVELOPER_DIR`, so the toolchain shims skip a host symlink the
///   sandbox cannot read — see [`probe_developer_dir`].
/// - `TMPDIR`, pointed **inside the project root** — see
///   [`project_tmpdir`], which is where the reasoning lives.
fn activated_env(
    baseline: &BTreeMap<String, String>,
    swift_bin: &Path,
    toolchain: &Toolchain,
    tmpdir: Option<&Path>,
) -> BTreeMap<String, String> {
    let mut activated = baseline.clone();
    if let Some(bin_dir) = swift_bin.parent().map(Path::to_string_lossy) {
        let existing = baseline.get("PATH").map(String::as_str).unwrap_or("");
        activated.insert("PATH".to_string(), format!("{bin_dir}:{existing}"));
    }
    if let Some(sdk) = &toolchain.sdk_path {
        activated.insert("SDKROOT".to_string(), sdk.clone());
    }
    if let Some(dev) = &toolchain.developer_dir {
        activated.insert("DEVELOPER_DIR".to_string(), dev.clone());
    }
    if let Some(tmp) = tmpdir {
        activated.insert("TMPDIR".to_string(), tmp.to_string_lossy().into_owned());
    }
    activated
}

/// A temporary directory **inside the project root**, for SwiftPM to
/// write its caches into.
///
/// **Found by running the build, and the constraint it satisfies is a
/// standing invariant rather than a preference.** With the toolchain
/// resolved, `swift build` failed next on
///
/// ```text
/// swift: error: couldn't create cache file
///   '/var/folders/__/…/T/xcrun_db-T2q5wfUP' (errno=Operation not permitted)
/// error: … couldNotFindTmpDir("/var/folders/__/…/T")
/// ```
///
/// The host's own `TMPDIR` is a per-user directory outside the project,
/// and the tempting fix — grant it read-write — is the one thing a
/// provider may not do. CLAUDE.md: *"Provider resolution must not widen
/// the policy. If activation would need write access outside the project
/// root, `up` fails naming the path rather than silently granting it."*
/// A provider that quietly granted write to `/var/folders/<user>/…` would
/// be handing the sandbox a host-global scratch space shared with every
/// other process of that user, which is exactly the widening that rule
/// exists to stop.
///
/// So the temp directory moves inside the project root, which is already
/// writable and already the sandbox's own. Under `.build/`, because
/// SwiftPM owns that directory, every Swift project's `.gitignore`
/// already covers it, and `swift package clean` disposes of it — devcroft
/// introduces no new path a user has to know about or clean up.
///
/// Created host-side at `up` rather than left to the sandbox: SwiftPM
/// wants `TMPDIR` to exist before it starts, and a provider materializing
/// something inside the project root is ordinary provisioning.
fn project_tmpdir(project_root: &Path) -> Result<PathBuf, ProviderError> {
    let dir = project_root.join(".build").join("devcroft-tmp");
    std::fs::create_dir_all(&dir).map_err(|e| {
        ProviderError::ResolutionFailed(format!(
            "creating the project-local temporary directory {}: {e}",
            dir.display()
        ))
    })?;
    Ok(dir)
}

/// Content fingerprint of `Package.swift` + `Package.resolved`, for
/// staleness detection. `provider::is_stale` (dispatched by provider
/// name) compares this against the fingerprint recorded at the last `up`.
///
/// Uses `capture::optional_file_part` for the lockfile so an **absent**
/// `Package.resolved` and a **present-but-empty** one stay distinct — a
/// lockfile appearing is itself a change, and that property is the reason
/// the helper exists.
pub fn package_fingerprint(project_root: &Path) -> Result<String, ProviderError> {
    ensure_package_present(project_root)?;
    let manifest = std::fs::read(project_root.join("Package.swift"))
        .map_err(|e| ProviderError::ResolutionFailed(format!("reading Package.swift: {e}")))?;
    let lock = capture::optional_file_part(&project_root.join("Package.resolved"));
    Ok(capture::fingerprint(&[&manifest, &lock]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape measured on Swift 6.1.2 / macOS 15, kept verbatim so a
    /// future toolchain that changes it fails here rather than in a
    /// sandbox that silently lost a grant.
    const REAL_TARGET_INFO: &str = r#"{
      "compilerVersion": "Apple Swift version 6.1.2",
      "target": { "triple": "arm64-apple-macosx15.0" },
      "paths": {
        "runtimeLibraryPaths": [
          "/Library/Developer/CommandLineTools/usr/lib/swift/macosx",
          "/usr/lib/swift"
        ],
        "runtimeLibraryImportPaths": [
          "/Library/Developer/CommandLineTools/usr/lib/swift/macosx"
        ],
        "runtimeResourcePath": "/Library/Developer/CommandLineTools/usr/lib/swift"
      }
    }"#;

    #[test]
    fn target_info_yields_the_paths_the_grants_are_built_from() {
        let t = parse_target_info(REAL_TARGET_INFO.as_bytes()).unwrap();
        assert_eq!(
            t.runtime_resource_path,
            "/Library/Developer/CommandLineTools/usr/lib/swift"
        );
        assert_eq!(t.runtime_library_paths.len(), 2);
        assert!(
            t.runtime_library_paths
                .iter()
                .any(|p| p == "/usr/lib/swift")
        );
        // The SDK is probed separately, not read from this output.
        assert_eq!(t.sdk_path, None);
    }

    /// **The decisive negative.** A `paths` block without
    /// `runtimeResourcePath` must fail rather than yield a toolchain with
    /// an empty resource path: the resource directory is what `swiftc`
    /// cannot compile without, so an empty string would compile a grant
    /// list that looks populated and produces a sandbox that cannot
    /// build. Failing at layer `provider` is legible; that is not.
    #[test]
    fn target_info_without_a_resource_path_is_refused() {
        let raw = br#"{"paths": {"runtimeLibraryPaths": ["/usr/lib/swift"]}}"#;
        match parse_target_info(raw) {
            Err(ProviderError::ResolutionFailed(msg)) => {
                assert!(
                    msg.contains("runtimeResourcePath"),
                    "the error must name the missing field; got: {msg}"
                );
            }
            other => panic!("expected ResolutionFailed, got {other:?}"),
        }
    }

    #[test]
    fn target_info_without_a_paths_block_is_refused() {
        match parse_target_info(br#"{"compilerVersion": "x"}"#) {
            Err(ProviderError::ResolutionFailed(msg)) => assert!(msg.contains("paths")),
            other => panic!("expected ResolutionFailed, got {other:?}"),
        }
    }

    /// The `swift` binary's own directory has to be in the grants: on
    /// macOS `/usr/bin/swift` is a shim and the toolchain lives under
    /// `/Library/Developer`, so granting only the library paths yields a
    /// sandbox that can read the toolchain and not invoke it.
    #[test]
    fn grants_include_the_binary_directory_and_are_deduplicated() {
        let t = parse_target_info(REAL_TARGET_INFO.as_bytes()).unwrap();
        let grants = t.grants(Path::new("/usr/bin/swift"));
        assert!(grants.contains(&"/usr/bin".to_string()));
        assert!(grants.contains(&"/usr/lib/swift".to_string()));

        let mut sorted = grants.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), grants.len(), "grants must be deduplicated");
    }

    /// The resource path and a library path are identical strings on some
    /// installs; the grant list must not carry the same path twice.
    #[test]
    fn grants_do_not_repeat_a_path_named_twice_by_the_toolchain() {
        let raw = br#"{"paths": {
            "runtimeLibraryPaths": ["/usr/lib/swift"],
            "runtimeResourcePath": "/usr/lib/swift"
        }}"#;
        let t = parse_target_info(raw).unwrap();
        let grants = t.grants(Path::new("/usr/bin/swift"));
        assert_eq!(
            grants.iter().filter(|p| *p == "/usr/lib/swift").count(),
            1,
            "got: {grants:?}"
        );
    }

    #[test]
    fn a_package_with_dependencies_is_detected() {
        let raw = br#"{"name": "x", "dependencies": [{"sourceControl": []}]}"#;
        assert!(parse_declares_dependencies(raw).unwrap());
    }

    #[test]
    fn a_package_without_dependencies_is_detected() {
        let raw = br#"{"name": "x", "dependencies": []}"#;
        assert!(!parse_declares_dependencies(raw).unwrap());
    }

    /// A schema with no `dependencies` key must fail rather than read as
    /// "no dependencies": that reading silently drops the lockfile
    /// requirement, which is the one guarantee this provider buys with
    /// the project-code execution it performs.
    #[test]
    fn a_description_without_a_dependencies_array_is_refused() {
        match parse_declares_dependencies(br#"{"name": "x"}"#) {
            Err(ProviderError::ResolutionFailed(msg)) => assert!(msg.contains("dependencies")),
            other => panic!("expected ResolutionFailed, got {other:?}"),
        }
    }

    /// Both directions of the conditional lockfile rule, in one place,
    /// because either half alone is satisfied by a wrong implementation:
    /// "always require" passes the first, "never require" passes the
    /// second.
    #[test]
    fn the_lockfile_is_required_only_when_dependencies_exist() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-swift-lock-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Dependencies, no lockfile: refused, with the remedy named.
        match ensure_lock_present(&dir, true) {
            Err(ProviderError::MissingLock { provider, hint }) => {
                assert_eq!(provider, "swift");
                assert_eq!(hint, "swift package resolve");
            }
            other => panic!("expected MissingLock, got {other:?}"),
        }

        // No dependencies, no lockfile: accepted. SwiftPM does not write
        // `Package.resolved` for such a package, so refusing here would
        // be a rejection with no remedy.
        assert!(ensure_lock_present(&dir, false).is_ok());

        // Dependencies with a lockfile: accepted.
        std::fs::write(dir.join("Package.resolved"), b"{}").unwrap();
        assert!(ensure_lock_present(&dir, true).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_package_manifest_names_its_own_remedy() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-swift-nopkg-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        match ensure_package_present(&dir) {
            Err(ProviderError::NoEnvironment { provider, hint }) => {
                assert_eq!(provider, "swift");
                assert_eq!(hint, "swift package init");
            }
            other => panic!("expected NoEnvironment, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The property `capture::optional_file_part` exists to preserve,
    /// asserted here because this provider is the third to claim it.
    #[test]
    fn a_lockfile_appearing_is_itself_a_change() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-swift-fp-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Package.swift"), b"// swift-tools-version:5.9").unwrap();

        let before = package_fingerprint(&dir).unwrap();
        std::fs::write(dir.join("Package.resolved"), b"").unwrap();
        let after = package_fingerprint(&dir).unwrap();
        assert_ne!(
            before, after,
            "an empty Package.resolved appearing must still count as a change"
        );

        std::fs::write(dir.join("Package.swift"), b"// swift-tools-version:6.0").unwrap();
        let edited = package_fingerprint(&dir).unwrap();
        assert_ne!(after, edited, "editing Package.swift must be a change");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `PATH` must lead with the toolchain devcroft actually probed, not
    /// merely contain it — otherwise a `swift` earlier on the sandbox's
    /// `PATH` would win and the grants would describe a different
    /// toolchain than the one that runs.
    #[test]
    fn activated_path_leads_with_the_probed_toolchain() {
        let baseline = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        let toolchain = parse_target_info(REAL_TARGET_INFO.as_bytes()).unwrap();
        let activated = activated_env(
            &baseline,
            Path::new("/opt/swift/bin/swift"),
            &toolchain,
            None,
        );
        assert!(
            activated
                .get("PATH")
                .unwrap()
                .starts_with("/opt/swift/bin:"),
            "got: {:?}",
            activated.get("PATH")
        );
    }

    #[test]
    fn sdkroot_is_set_only_when_the_host_has_an_sdk() {
        let baseline = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);
        let mut toolchain = parse_target_info(REAL_TARGET_INFO.as_bytes()).unwrap();
        assert!(
            !activated_env(&baseline, Path::new("/usr/bin/swift"), &toolchain, None)
                .contains_key("SDKROOT"),
            "a host with no SDK must not get an empty SDKROOT"
        );

        toolchain.sdk_path = Some("/SDKs/MacOSX.sdk".to_string());
        assert_eq!(
            activated_env(&baseline, Path::new("/usr/bin/swift"), &toolchain, None).get("SDKROOT"),
            Some(&"/SDKs/MacOSX.sdk".to_string())
        );
    }
}

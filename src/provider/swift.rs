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

        // The one call that runs project code. Two things are read out
        // of it: whether the package declares dependencies, and whether
        // it links any Apple framework.
        let description = describe_package(&swift_bin, project_root)?;

        // Before anything else this provider could do for the project:
        // is this provider even the right answer for it?
        ensure_macos_dependent(project_root, &description)?;
        ensure_lock_present(project_root, description.declares_dependencies)?;

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

/// Ask SwiftPM for the package description, and read two things out of
/// it.
///
/// **This is the criterion-4 violation, confined to one call.** It is
/// kept because the alternative is worse: skipping it would leave
/// `Package.resolved` unenforced, dropping criterion 2 (restorable
/// lockfile) as well as 4, and leaving the provider with no
/// reproducibility claim at all. If project code is going to run, it
/// should at least buy something.
///
/// `--disable-sandbox` is deliberately **not** passed. SwiftPM's own
/// manifest sandbox blocks writes on macOS, which is a partial mitigation
/// devcroft has no reason to switch off, and passing the flag would make
/// devcroft responsible for a weakening it did not need.
fn describe_package(
    swift_bin: &Path,
    project_root: &Path,
) -> Result<PackageDescription, ProviderError> {
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

    parse_package_description(&output.stdout)
}

/// The two fields consumed from `dump-package`'s output.
///
/// Deliberately narrow. The full description carries targets, products,
/// platforms and more, and every one of them is a thing this provider
/// would then have to keep working across SwiftPM schema versions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PackageDescription {
    pub(super) declares_dependencies: bool,
    /// Apple frameworks the package links. A framework is an Apple-only
    /// linkage concept, so any entry here is conclusive evidence that no
    /// Linux closure can build this package.
    pub(super) linked_frameworks: Vec<String>,
}

pub(super) fn parse_package_description(raw: &[u8]) -> Result<PackageDescription, ProviderError> {
    let parsed: serde_json::Value = serde_json::from_slice(raw).map_err(|e| {
        ProviderError::ResolutionFailed(format!("parsing `swift package dump-package` output: {e}"))
    })?;

    let declares_dependencies = match parsed.get("dependencies") {
        Some(serde_json::Value::Array(deps)) => !deps.is_empty(),
        // A description with no `dependencies` key at all is a schema
        // devcroft does not recognize. Treated as "no dependencies"
        // would silently drop the lockfile requirement, so it fails
        // instead — the same rule `nix.rs` follows for an unparseable
        // `print-dev-env`.
        _ => {
            return Err(ProviderError::ResolutionFailed(
                "`swift package dump-package` output has no `dependencies` array; \
                 this SwiftPM version is not one devcroft can read"
                    .to_string(),
            ));
        }
    };

    // Measured shape (Swift 6.1.2):
    //   "settings": [{"kind": {"linkedFramework": {"_0": "AppKit"}},
    //                 "tool": "linker"}]
    // Read defensively rather than with a typed struct: unlike
    // `dependencies` above, a schema change here must not fail `up` — a
    // missing framework signal falls through to the source scan, which
    // is the more general evidence anyway.
    let mut linked_frameworks = Vec::new();
    if let Some(targets) = parsed.get("targets").and_then(|t| t.as_array()) {
        for target in targets {
            let Some(settings) = target.get("settings").and_then(|s| s.as_array()) else {
                continue;
            };
            for setting in settings {
                if let Some(name) = setting
                    .get("kind")
                    .and_then(|k| k.get("linkedFramework"))
                    .and_then(|f| f.get("_0"))
                    .and_then(|n| n.as_str())
                {
                    linked_frameworks.push(name.to_string());
                }
            }
        }
    }
    linked_frameworks.sort();
    linked_frameworks.dedup();

    Ok(PackageDescription {
        declares_dependencies,
        linked_frameworks,
    })
}

/// Read the toolchain's own paths from `swift -print-target-info`.
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

/// Modules that exist only on Apple platforms, so an unguarded import of
/// one is conclusive evidence that no Linux closure can build the package.
///
/// **`Foundation` and `Dispatch` are deliberately absent**, and that
/// omission is the whole accuracy of this list. Both ship on Linux via
/// swift-corelibs, so treating them as Apple-only — the obvious mistake —
/// would classify essentially every Swift package as macOS-dependent and
/// turn the gate below into a rubber stamp.
const APPLE_ONLY_MODULES: &[&str] = &[
    "AppKit",
    "UIKit",
    "SwiftUI",
    "Cocoa",
    "CoreGraphics",
    "CoreData",
    "CoreML",
    "CoreImage",
    "CoreAudio",
    "CoreBluetooth",
    "CoreLocation",
    "AVFoundation",
    "Metal",
    "MetalKit",
    "QuartzCore",
    "WebKit",
    "ObjectiveC",
    "Security",
    "IOKit",
    "Carbon",
    "ServiceManagement",
    "UserNotifications",
    "StoreKit",
    "CloudKit",
    "GameKit",
    "SpriteKit",
    "SceneKit",
    "ARKit",
    "HealthKit",
    "MapKit",
    "PhotosUI",
    "Vision",
    "NaturalLanguage",
    "Combine",
];

/// An unguarded import of an Apple-only module, with where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AppleImport {
    pub(super) module: String,
    pub(super) file: String,
}

/// Scan the package's Swift sources for imports of Apple-only modules
/// that are **not** inside a conditional-compilation block.
///
/// **The guard test is the difference between "needs macOS" and "supports
/// macOS"**, and getting it wrong in either direction breaks the gate.
/// A package that writes
///
/// ```swift
/// #if canImport(AppKit)
/// import AppKit
/// #endif
/// ```
///
/// is *portable* — it has an Apple branch and a non-Apple one — so it is
/// exactly the case that should go to flox or nix, and counting that
/// import would send it to `swift` instead. An import at top level with
/// no `#if` around it cannot compile on Linux at all.
///
/// Tracks nesting rather than matching the nearest `#if`, because a
/// conditional import inside an outer conditional block is still
/// conditional. Only `#if` conditions that actually test availability
/// (`canImport(...)`, `os(...)`, `targetEnvironment(...)`) open a guarded
/// region; an `#if DEBUG` around an import guards nothing about the
/// platform.
///
/// Reads no more than the package's own sources, executes nothing, and is
/// bounded by the file tree — so unlike the `dump-package` call above it
/// adds no new exposure.
pub(super) fn scan_apple_only_imports(project_root: &Path) -> Vec<AppleImport> {
    let mut found = Vec::new();
    let mut stack = vec![project_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                // `.build` holds checked-out dependency sources, whose
                // imports say nothing about *this* package's platform
                // needs. Hidden directories are skipped for the same
                // reason `.git` is uninteresting.
                if name == ".build" || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "swift") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    collect_unguarded_imports(&text, &path.display().to_string(), &mut found);
                }
            }
        }
    }
    found.sort_by(|a, b| (&a.module, &a.file).cmp(&(&b.module, &b.file)));
    found.dedup();
    found
}

/// The line-by-line half of [`scan_apple_only_imports`], separated so it
/// can be tested on text without a file tree.
pub(super) fn collect_unguarded_imports(text: &str, file: &str, out: &mut Vec<AppleImport>) {
    let mut guard_depth = 0usize;
    let mut conditional_depth = 0usize;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(condition) = line.strip_prefix("#if") {
            conditional_depth += 1;
            if condition.contains("canImport(")
                || condition.contains("os(")
                || condition.contains("targetEnvironment(")
            {
                guard_depth += 1;
            }
            continue;
        }
        if line.starts_with("#endif") {
            conditional_depth = conditional_depth.saturating_sub(1);
            // Only close a guarded region when the block being closed
            // opened one. Tracking both depths is what keeps an
            // `#if DEBUG` nested inside an `#if canImport(...)` from
            // ending the guard early.
            if guard_depth > conditional_depth {
                guard_depth = conditional_depth;
            }
            continue;
        }
        if guard_depth > 0 {
            continue;
        }
        let Some(rest) = line.strip_prefix("import ") else {
            continue;
        };
        // `import struct Foundation.Data` — take the module, which is the
        // last dotted path's first component after any declaration kind.
        let module = rest
            .split_whitespace()
            .last()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(';');
        if APPLE_ONLY_MODULES.contains(&module) {
            out.push(AppleImport {
                module: module.to_string(),
                file: file.to_string(),
            });
        }
    }
}

/// Refuse `swift` for a package a qualifying provider already covers.
///
/// **This is the narrowing that makes shipping a test-failing provider
/// defensible at all.** `docs/decisions.md` §1 justifies `swift` on the
/// grounds that Swift users otherwise get nothing — but that is only true
/// for packages depending on Apple frameworks. A portable Swift package
/// builds perfectly well from a nix or flox closure, where it gets the
/// closure tier, a hook-free activation, and no host-side execution of
/// `Package.swift`. Offering it the weaker provider buys the user nothing
/// and costs them all three.
///
/// So the provider is accepted only on positive evidence that no closure
/// can serve the project:
///
/// - a linked Apple framework, which is conclusive; or
/// - an unguarded import of an Apple-only module, which cannot compile on
///   Linux.
///
/// **`platforms:` is deliberately not evidence**, and this is the subtlety
/// that would otherwise make the gate useless. `platforms: [.macOS(.v13)]`
/// only sets minimum versions for Apple platforms — SwiftPM on Linux
/// ignores it entirely — so thousands of portable packages declare it.
/// Using it here would accept nearly everything and narrow nothing.
///
/// **A heuristic, and it says so when it refuses.** `docs/decisions.md`
/// §1's criterion 6 is hostile to preconditions that cannot be checked
/// cleanly, and this one can be wrong: a macOS-only package that reaches
/// Apple APIs some other way is refused despite having no alternative.
/// The refusal therefore names exactly what was searched for, so a user
/// who is on the wrong side of it can see why rather than guess. The
/// asymmetry is deliberate — a wrong refusal sends someone to a *better*
/// provider, while a wrong acceptance silently downgrades their guarantee
/// and runs their code on the host.
fn ensure_macos_dependent(
    project_root: &Path,
    description: &PackageDescription,
) -> Result<(), ProviderError> {
    if !description.linked_frameworks.is_empty() {
        return Ok(());
    }
    let imports = scan_apple_only_imports(project_root);
    if !imports.is_empty() {
        return Ok(());
    }
    Err(ProviderError::CoveredByQualifiedProvider {
        provider: "swift",
        reason: format!(
            "nothing in this package requires Apple platforms, so a closure-tier provider \
             covers it — and covers it better (reproducible across machines, and `up` does \
             not run Package.swift on the host). devcroft looked for a linked Apple \
             framework and for an unguarded `import` of an Apple-only module ({} of them, \
             including AppKit, UIKit, SwiftUI and Metal) under {}, and found neither; note \
             that `platforms: [.macOS(...)]` is not evidence, since SwiftPM ignores it on \
             Linux. Use `provider = \"nix\"` or `provider = \"flox\"` with the swift \
             package. If this project really is macOS-only, that is a gap in this check \
             worth reporting rather than working around",
            APPLE_ONLY_MODULES.len(),
            project_root.display()
        ),
    })
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
        assert!(
            parse_package_description(raw)
                .unwrap()
                .declares_dependencies
        );
    }

    #[test]
    fn a_package_without_dependencies_is_detected() {
        let raw = br#"{"name": "x", "dependencies": []}"#;
        assert!(
            !parse_package_description(raw)
                .unwrap()
                .declares_dependencies
        );
    }

    /// The measured shape, kept verbatim so a SwiftPM schema change fails
    /// here rather than silently emptying the strongest signal the gate
    /// has.
    #[test]
    fn linked_frameworks_are_read_from_target_settings() {
        let raw = br#"{"dependencies": [], "targets": [
            {"name": "app", "settings": [
              {"kind": {"linkedFramework": {"_0": "AppKit"}}, "tool": "linker"},
              {"kind": {"linkedFramework": {"_0": "Metal"}}, "tool": "linker"}
            ]}]}"#;
        let d = parse_package_description(raw).unwrap();
        assert_eq!(d.linked_frameworks, vec!["AppKit", "Metal"]);
    }

    /// A schema change in `settings` must not fail `up` — unlike
    /// `dependencies`, this signal has a fallback (the source scan), so
    /// failing closed here would refuse projects over a field devcroft
    /// merely stopped recognizing.
    #[test]
    fn an_unreadable_settings_shape_yields_no_frameworks_rather_than_failing() {
        let raw = br#"{"dependencies": [], "targets": [{"name": "a", "settings": "surprise"}]}"#;
        let d = parse_package_description(raw).unwrap();
        assert!(d.linked_frameworks.is_empty());
    }

    /// A schema with no `dependencies` key must fail rather than read as
    /// "no dependencies": that reading silently drops the lockfile
    /// requirement, which is the one guarantee this provider buys with
    /// the project-code execution it performs.
    #[test]
    fn a_description_without_a_dependencies_array_is_refused() {
        match parse_package_description(br#"{"name": "x"}"#) {
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
    fn imports(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        collect_unguarded_imports(text, "t.swift", &mut out);
        out.into_iter().map(|i| i.module).collect()
    }

    /// A top-level Apple import cannot compile on Linux, so it is
    /// conclusive evidence the project needs this provider.
    #[test]
    fn an_unguarded_apple_import_counts() {
        assert_eq!(imports("import AppKit\nprint(1)\n"), vec!["AppKit"]);
        assert_eq!(imports("import SwiftUI\n"), vec!["SwiftUI"]);
    }

    /// **The distinction the whole gate rests on.** A `canImport` guard
    /// means the package is *portable* with an Apple branch — exactly the
    /// case that should go to a closure provider — so counting it would
    /// send portable packages to the weaker provider, which is the thing
    /// this gate exists to prevent.
    #[test]
    fn a_guarded_apple_import_does_not_count() {
        assert!(imports("#if canImport(AppKit)\nimport AppKit\n#endif\n").is_empty());
        assert!(imports("#if os(macOS)\nimport Cocoa\n#endif\n").is_empty());
    }

    /// `#if DEBUG` guards nothing about the platform, so an import inside
    /// one is still unguarded for this purpose. Without this the gate
    /// would accept a portable package that happens to wrap an import in
    /// any conditional at all.
    #[test]
    fn a_non_platform_conditional_is_not_a_guard() {
        assert_eq!(
            imports("#if DEBUG\nimport AppKit\n#endif\n"),
            vec!["AppKit"]
        );
    }

    /// Nesting: an inner non-platform block must not end the outer
    /// platform guard when it closes.
    #[test]
    fn an_inner_conditional_does_not_end_an_outer_platform_guard() {
        let text = "#if canImport(UIKit)\n#if DEBUG\nimport UIKit\n#endif\nimport UIKit\n#endif\n";
        assert!(
            imports(text).is_empty(),
            "both imports are inside the canImport guard; got {:?}",
            imports(text)
        );
    }

    /// Foundation and Dispatch ship on Linux via swift-corelibs. Treating
    /// them as Apple-only is the mistake that would turn this gate into a
    /// rubber stamp, since nearly every Swift file imports Foundation.
    #[test]
    fn linux_available_modules_are_not_apple_signals() {
        assert!(imports("import Foundation\nimport Dispatch\n").is_empty());
    }

    #[test]
    fn a_submodule_import_is_matched_on_its_module() {
        assert_eq!(
            imports("import struct CoreData.NSManagedObject\n"),
            vec!["CoreData"]
        );
    }

    /// The gate, end to end, on a real file tree. Both directions, since
    /// "always accept" and "always refuse" each pass one half alone.
    #[test]
    fn the_gate_accepts_only_packages_a_closure_cannot_serve() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-swift-gate-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Sources/app")).unwrap();

        // Portable: refused, and the refusal must point at the providers
        // that serve it better.
        std::fs::write(
            dir.join("Sources/app/main.swift"),
            "import Foundation\nprint(1)\n",
        )
        .unwrap();
        let portable = PackageDescription::default();
        match ensure_macos_dependent(&dir, &portable) {
            Err(ProviderError::CoveredByQualifiedProvider { provider, reason }) => {
                assert_eq!(provider, "swift");
                assert!(reason.contains("nix") && reason.contains("flox"));
                assert!(
                    reason.contains("platforms"),
                    "the refusal must pre-empt the obvious objection — that the package \
                     declares a macOS platform — since that is not evidence; got: {reason}"
                );
            }
            other => panic!("a portable package must be refused, got {other:?}"),
        }

        // A linked framework alone is conclusive, with no Apple import.
        let framework = PackageDescription {
            declares_dependencies: false,
            linked_frameworks: vec!["AppKit".to_string()],
        };
        assert!(ensure_macos_dependent(&dir, &framework).is_ok());

        // An unguarded Apple import alone is conclusive, with no
        // framework.
        std::fs::write(
            dir.join("Sources/app/main.swift"),
            "import AppKit\nprint(1)\n",
        )
        .unwrap();
        assert!(ensure_macos_dependent(&dir, &portable).is_ok());

        // Guarded again: back to refused, so the guard test is live
        // through the real file scan and not only in `imports()`.
        std::fs::write(
            dir.join("Sources/app/main.swift"),
            "#if canImport(AppKit)\nimport AppKit\n#endif\n",
        )
        .unwrap();
        assert!(
            ensure_macos_dependent(&dir, &portable).is_err(),
            "a package whose only Apple import is guarded is portable"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dependency sources under `.build` are other people's packages, and
    /// their imports say nothing about what *this* package needs. Without
    /// this exclusion, one dependency importing AppKit would qualify
    /// every project that had ever run `swift build`.
    #[test]
    fn imports_from_checked_out_dependencies_are_ignored() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-swift-gatedeps-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Sources/app")).unwrap();
        std::fs::create_dir_all(dir.join(".build/checkouts/other/Sources")).unwrap();
        std::fs::write(dir.join("Sources/app/main.swift"), "import Foundation\n").unwrap();
        std::fs::write(
            dir.join(".build/checkouts/other/Sources/x.swift"),
            "import AppKit\n",
        )
        .unwrap();

        assert!(
            scan_apple_only_imports(&dir).is_empty(),
            "a dependency's AppKit import must not qualify this package"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

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

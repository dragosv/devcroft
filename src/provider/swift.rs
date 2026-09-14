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
        ensure_macos_host()?;
        ensure_package_present(project_root)?;
        let swift_bin = resolve_on_path("swift").ok_or(ProviderError::MissingBinary {
            provider: "swift",
            hint: "devcroft doctor",
        })?;

        // Data only: no project file is evaluated to answer this.
        let toolchain = probe_toolchain(&swift_bin)?;

        // Whether this provider is the *best* answer for this project is
        // `up`'s to say, as advice (`apple_evidence_advice`); the manifest
        // asked for swift, and resolution honours what it was asked.

        let baseline = capture::canonical_base_env()?;
        let dirs = ProviderDirs::create(project_root)?;
        dirs.write_shim(&swift_bin)?;
        let activated = activated_env(&baseline, &swift_bin, &toolchain, Some(&dirs));

        Ok(Resolution {
            env: capture::changed_env(&baseline, &activated),
            unset: capture::unset_env(&baseline, &activated),
            read_only_grants: toolchain.grants(&swift_bin),
            // Swift has no service concept. Declared rather than left
            // empty for the same reason `nix.rs` states: `up` must be
            // able to tell "cannot do services" from "can, none
            // declared", so a project asking for them fails loudly.
            services: super::ServiceSupport::Unsupported,
            // **False, and getting here took a correction worth
            // recording.** An earlier version of this provider called
            // `swift package dump-package` to learn whether the package
            // declared dependencies, so it could require
            // `Package.resolved` — and `dump-package` compiles and runs
            // `Package.swift`. It disclosed that faithfully and shipped
            // the violation.
            //
            // The violation was avoidable, and the fix was to do less.
            // devcroft resolves the *toolchain*, which comes entirely
            // from `xcode-select`/`xcrun` with no project file involved.
            // Dependency resolution belongs *inside* the sandbox, at
            // `swift build` time, where a hostile manifest is confined by
            // the policy the project declared. Once devcroft stops
            // materializing dependencies host-side it has no reason to
            // read the package graph at all, which retires the execution
            // and the lockfile precondition together.
            //
            // The result is the **cleanest criterion-4 pass of any
            // provider devcroft has**: not "a hook-free entry point was
            // found", but "no project file is opened".
            ran_activation_hook: false,
            // Nothing to defer: no project script is captured, because
            // none is read.
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
        //
        // **Under Xcode, the grant is the app bundle's `Contents`, not
        // `Contents/Developer`.** Measured with Xcode 26 / Swift 6.2.4 (the
        // branch had only been run against Command Line Tools, where no
        // `xcodebuild` exists): `swift build` runs `xcodebuild`, which is
        // linked against `Xcode.app/Contents/SharedFrameworks/DVT*.framework`
        // — a sibling of `Developer`, not inside it — and the build died at
        // dyld with "blocked by sandbox" before compiling anything. Granting
        // `Contents` read-only covers `Developer`, `Frameworks`,
        // `SharedFrameworks` and `PlugIns`, which is what "the toolchain"
        // means when the toolchain is Xcode. Read-only, so nothing inside
        // the sandbox can alter it; still host-linked, which is the
        // artifact tier's stated cost, not a new one.
        if let Some(dev) = &self.developer_dir {
            let dev_path = Path::new(dev);
            let bundle_contents = dev_path
                .file_name()
                .is_some_and(|n| n == "Developer")
                .then(|| dev_path.parent())
                .flatten()
                .filter(|c| c.file_name().is_some_and(|n| n == "Contents"));
            match bundle_contents {
                Some(contents) => {
                    out.push(contents.to_string_lossy().into_owned());
                    // Where `xcodebuild` reads the record that the Xcode
                    // license was accepted. Measured: with the license
                    // accepted on the host, a sandboxed `swift build` still
                    // said "You have not agreed to the Xcode license
                    // agreements" — it could not read this file, and treats
                    // unreadable as unaccepted. A single file, read-only;
                    // `doctor` checks the same file from outside so the
                    // host-side half of that failure is named before `up`.
                    out.push(XCODE_LICENSE_RECORD.to_string());
                    // `xcodebuild` loads every plug-in under
                    // `Contents/PlugIns` at startup, and
                    // `IDEiOSSupportCore` links `MobileDevice.framework`,
                    // which lives under `/Library/Apple` — the sealed
                    // system volume holds only a symlink to it. Measured:
                    // ungranted, `xcodebuild -find swift` fails its plug-in
                    // scan ("Loading a plug-in failed") and `xcrun` reports
                    // that swift cannot be located at all. Read-only; the
                    // directory is Apple's own device-support software.
                    if Path::new(APPLE_SUPPORT_LIBRARY).is_dir() {
                        out.push(APPLE_SUPPORT_LIBRARY.to_string());
                    }
                }
                None => out.push(dev.clone()),
            }
        }
        if let Some(dir) = swift_bin.parent() {
            out.push(dir.to_string_lossy().into_owned());
        }
        if let Some(dir) = host_shell_dir() {
            out.push(dir);
            // macOS's `/bin/sh` is a selector that reads `/var/select/sh`
            // to decide which shell to be. Ungranted, `sh -c` from inside
            // the sandbox fails with "Error opening /private/var/select/sh:
            // Operation not permitted" — and `xcrun` runs `sh -c
            // 'xcodebuild -find swift'` on every invocation, so under Xcode
            // that was fatal ("Failed to locate 'swift'"). Measured; the
            // same message was already known as noise in a devbox postgres
            // log (docs/known-gaps.md) — here it stops the build. Granted
            // by its lexical spelling so the backend emits both
            // (`fix-symlinked-grant-spelling`); a directory of one symlink.
            if Path::new(HOST_SHELL_SELECTOR).is_dir() {
                out.push(HOST_SHELL_SELECTOR.to_string());
            }
        }
        out.sort();
        out.dedup();
        out.retain(|p| !p.is_empty());
        out
    }
}

/// Where macOS's `/bin/sh` selector reads which shell to run. See
/// `Toolchain::grants`.
const HOST_SHELL_SELECTOR: &str = "/var/select";

/// Apple's device-support frameworks (`MobileDevice.framework` and
/// siblings), reached from `/System/Library/PrivateFrameworks` through a
/// symlink. See `Toolchain::grants`.
const APPLE_SUPPORT_LIBRARY: &str = "/Library/Apple";

/// The plist `xcodebuild` consults for license acceptance. See
/// `Toolchain::grants` for why it is granted, and `doctor` for why it is
/// checked host-side too.
pub(crate) const XCODE_LICENSE_RECORD: &str = "/Library/Preferences/com.apple.dt.Xcode.plist";

/// Refuse this provider anywhere but macOS.
///
/// **Swift exists on Linux; an Xcode-backed provider does not.** Every
/// path this module takes — `xcode-select`, `xcrun --show-sdk-path`, the
/// SDK, the framework evidence the gate looks for — is Apple-specific.
/// Running under the same provider name on Linux would silently resolve a
/// different toolchain with a different guarantee, which is the worst
/// available outcome: the manifest would say `swift` on both machines and
/// mean two different things.
///
/// Fails closed, at layer `provider`, naming the platform. On Linux the
/// answer is a closure provider, which is also where a Swift toolchain
/// there actually comes from.
fn ensure_macos_host() -> Result<(), ProviderError> {
    if cfg!(target_os = "macos") {
        return Ok(());
    }
    Err(ProviderError::CoveredByQualifiedProvider {
        provider: "swift",
        reason: "this provider resolves an Xcode or Command Line Tools toolchain and runs \
                 only on macOS. Swift itself exists on this platform, but it comes from a \
                 package set — use `provider = \"nix\"` or `provider = \"flox\"` with the \
                 swift package, which also gives you the closure tier"
            .to_string(),
    })
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
/// - `SWIFTPM_BUILD_DIR`, `TMPDIR`, `CLANG_MODULE_CACHE_PATH` and
///   `xcrun_db`, all pointed **inside the project root** — see
///   [`ProviderDirs`] and the comment on each below for what each was
///   measured to move. The `swift` on `PATH` is the shim
///   [`ProviderDirs::write_shim`] writes, for the two flags that have no
///   variable.
fn activated_env(
    baseline: &BTreeMap<String, String>,
    swift_bin: &Path,
    toolchain: &Toolchain,
    dirs: Option<&ProviderDirs>,
) -> BTreeMap<String, String> {
    let mut activated = baseline.clone();
    if let Some(bin_dir) = swift_bin.parent().map(Path::to_string_lossy) {
        let existing = baseline.get("PATH").map(String::as_str).unwrap_or("");
        // The shim's directory first, then the toolchain's: `swift` on
        // PATH is devcroft's wrapper, everything else (`swiftc`, `xcrun`,
        // `clang`) is the toolchain's own binary.
        let path = match dirs {
            Some(d) => format!("{}:{bin_dir}:{existing}", d.bin.display()),
            None => format!("{bin_dir}:{existing}"),
        };
        activated.insert("PATH".to_string(), path);
    }
    if let Some(sdk) = &toolchain.sdk_path {
        activated.insert("SDKROOT".to_string(), sdk.clone());
    }
    if let Some(dev) = &toolchain.developer_dir {
        activated.insert("DEVELOPER_DIR".to_string(), dev.clone());
    }
    if let Some(d) = dirs {
        // Every cache and scratch lever the toolchain has, pointed inside
        // the project, so a build needs no grant outside it. Each was
        // found by a build failing without it and measured to work —
        // under Xcode 26 / Swift 6.2.4, with only the project root
        // granted:
        //
        // - `SWIFTPM_BUILD_DIR`: SwiftPM's scratch and products; set to
        //   the conventional `.build` explicitly rather than left to
        //   default, so it is data in `policy --render`'s neighbour
        //   (`exec -- env`) and not an assumption.
        // - `TMPDIR`: everything that reads it — the Swift driver's
        //   temporary VFS overlays, SwiftPM's manifest compile scratch.
        //   Under `.devcroft`, not `.build`: a `swift package reset` or an
        //   `rm -rf .build` from inside used to delete the sandbox's own
        //   temporary directory and every later build failed with
        //   `couldNotFindTmpDir` (measured).
        // - `CLANG_MODULE_CACHE_PATH`: clang's, and through it the Swift
        //   frontend's, module cache. Without it the *manifest* compile —
        //   whose flags SwiftPM chooses, not the user — wrote to the
        //   Darwin per-user cache directory (`_CS_DARWIN_USER_CACHE_DIR`,
        //   `/var/folders/…/C`), which no `-module-cache-path` the user
        //   can pass reaches, and failed with "unable to load standard
        //   library". This variable is what removed the write grant on
        //   that directory from the sample's manifest.
        // - `xcrun_db` (lowercase — libxcrun's own name for it): where
        //   `xcrun` keeps its lookup cache, otherwise
        //   `_CS_DARWIN_USER_TEMP_DIR` again, and two "couldn't create
        //   cache file" lines on every build. Not documented; read out of
        //   `libxcrun.dylib`'s strings, then measured.
        //
        // What has **no** lever and needs the shim instead: SwiftPM's own
        // sandbox (`--disable-sandbox`) and its package cache
        // (`--cache-path`) — `strings swift-build | grep ^SWIFTPM_` lists
        // neither, and the branch that first measured this said so.
        activated.insert(
            "SWIFTPM_BUILD_DIR".to_string(),
            d.project_root.join(".build").to_string_lossy().into_owned(),
        );
        activated.insert("TMPDIR".to_string(), d.tmp.to_string_lossy().into_owned());
        activated.insert(
            "CLANG_MODULE_CACHE_PATH".to_string(),
            d.cache.join("clang").to_string_lossy().into_owned(),
        );
        activated.insert(
            "xcrun_db".to_string(),
            d.cache.join("xcrun_db").to_string_lossy().into_owned(),
        );
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

/// Files that only exist in a project building an Apple deliverable.
///
/// **These are evidence about the *product*, not the source**, and that
/// distinction is why they were added. A Mac application whose Swift is
/// entirely `Foundation` still cannot be produced by a Linux closure: the
/// app bundle, the entitlements, the code signature and `xcodebuild` are
/// all Apple-side. Judging such a project only by its imports refused it
/// and sent the user to flox, which cannot sign code or build a bundle —
/// a wrong refusal with no remedy.
///
/// Matched by file name or extension, so this stays a filesystem check:
/// no parsing, no execution, cheap enough for `init` to run.
fn is_apple_project_artifact(name: &str) -> bool {
    const APPLE_EXTENSIONS: &[&str] = &[
        ".entitlements",
        ".xcodeproj",
        ".xcworkspace",
        ".xcassets",
        ".storyboard",
        ".xib",
        ".xcconfig",
        ".appex",
    ];
    name == "Info.plist"
        || name == "PrivacyInfo.xcprivacy"
        || APPLE_EXTENSIONS.iter().any(|e| name.ends_with(e))
}

/// Everything the filesystem says about whether this package needs Apple
/// platforms.
///
/// Collected in one walk because both halves answer the same question and
/// both are needed by the same two callers (`up`'s gate, and `init`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct AppleEvidence {
    /// Unguarded imports of Apple-only modules — the source needs Apple.
    pub(super) imports: Vec<AppleImport>,
    /// Apple project artifacts — the *deliverable* needs Apple, even where
    /// the source would compile anywhere.
    pub(super) artifacts: Vec<String>,
    /// Apple frameworks named in `Package.swift`.
    ///
    /// **Read as text, never by evaluating the manifest.** An earlier
    /// version took these from `swift package dump-package`, which is
    /// authoritative — and which compiles and runs `Package.swift`. The
    /// authority was not worth the execution: this provider's whole
    /// criterion-4 position is that it opens no project file, and a
    /// signal costing that position costs more than it gives.
    ///
    /// The trade is bounded and stated: a framework name computed during
    /// manifest evaluation is missed. That is a false negative in a gate
    /// whose false negatives send the user to a *better* provider, and it
    /// is one of three independent signals rather than the only one.
    pub(super) frameworks: Vec<String>,
}

impl AppleEvidence {
    pub(super) fn is_empty(&self) -> bool {
        self.imports.is_empty() && self.artifacts.is_empty() && self.frameworks.is_empty()
    }
}

/// Scan the package for evidence that it needs Apple platforms: imports
/// of Apple-only modules that are **not** inside a conditional-compilation
/// block, and Apple project artifacts.
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
pub(super) fn scan_apple_evidence(project_root: &Path) -> AppleEvidence {
    let mut evidence = AppleEvidence::default();
    let mut stack = vec![project_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if is_apple_project_artifact(&name) {
                evidence.artifacts.push(path.display().to_string());
                // An `.xcodeproj` is a directory; recording it is enough,
                // and descending into it would add nothing but noise.
                if path.is_dir() {
                    continue;
                }
            }
            if path.is_dir() {
                // `.build` holds checked-out dependency sources, whose
                // imports say nothing about *this* package's platform
                // needs. Hidden directories are skipped for the same
                // reason `.git` is uninteresting.
                if name == ".build" || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "swift")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                if name == "Package.swift" {
                    evidence
                        .frameworks
                        .extend(frameworks_named_in_manifest(&text));
                } else {
                    collect_unguarded_imports(
                        &text,
                        &path.display().to_string(),
                        &mut evidence.imports,
                    );
                }
            }
        }
    }
    evidence
        .imports
        .sort_by(|a, b| (&a.module, &a.file).cmp(&(&b.module, &b.file)));
    evidence.imports.dedup();
    evidence.artifacts.sort();
    evidence.artifacts.dedup();
    evidence.frameworks.sort();
    evidence.frameworks.dedup();
    evidence
}

/// Apple frameworks named in a `Package.swift`, read as text.
///
/// Covers the two spellings that appear in real manifests:
/// `.linkedFramework("AppKit")`, and `-framework` followed by a name
/// inside `.unsafeFlags([...])`. Both are string literals in the source,
/// so reading them requires no evaluation.
pub(super) fn frameworks_named_in_manifest(text: &str) -> Vec<String> {
    fn literal_after(haystack: &str, at: usize) -> Option<String> {
        let rest = haystack.get(at..)?;
        let open = rest.find('"')? + 1;
        let close = rest.get(open..)?.find('"')? + open;
        let name = rest.get(open..close)?;
        (!name.is_empty()).then(|| name.to_string())
    }

    let mut out = Vec::new();
    for needle in [".linkedFramework(", "\"-framework\""] {
        let mut from = 0usize;
        while let Some(idx) = text.get(from..).and_then(|t| t.find(needle)) {
            let at = from + idx + needle.len();
            if let Some(name) = literal_after(text, at) {
                out.push(name);
            }
            from = at;
        }
    }
    out
}

/// The line-by-line half of [`scan_apple_evidence`], separated so it
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

/// What the evidence scan has to say about a project that shows no
/// Apple dependency: `None` where evidence exists, otherwise the advice.
///
/// **Advice, not a gate — and that is a reversal of this provider's
/// first cut, on review.** The scan reads the tree for a linked Apple
/// framework, an unguarded `import` of an Apple-only module, or an Apple
/// project artifact. It is good at *recommending* — `init` ranks swift
/// below every closure provider on it — and bad as a *refusal*: it
/// cannot prove nix or flox can actually build the project; it misses an
/// Apple-native project with an unusual layout; it accepts a project on
/// the strength of a stale `Info.plist`; and it turns the manifest's
/// explicit `provider = "swift"` into an inference that can go false as
/// the project evolves. So `up` honours the manifest and says this once,
/// with the closure-tier alternative named. A project that wants the
/// scan to refuse opts in with `[env] require_native_apple_evidence =
/// true`, in the committed file.
pub fn apple_evidence_advice(project_root: &Path) -> Option<String> {
    if !scan_apple_evidence(project_root).is_empty() {
        return None;
    }
    Some(format!(
        "nothing in this project requires Apple platforms, so a closure-tier provider \
         would cover it — and cover it better (reproducible across machines, and `up` does \
         not run Package.swift on the host). devcroft looked under {} for a linked Apple \
         framework, an unguarded `import` of an Apple-only module ({} of them, including \
         AppKit, SwiftUI, Metal and Security), and Apple project artifacts (Info.plist, \
         .entitlements, .xcodeproj, .xcassets), and found none; note that \
         `platforms: [.macOS(...)]` is not evidence, since SwiftPM ignores it on Linux. \
         `provider = \"nix\"` or `provider = \"flox\"` with the swift package is the \
         closure-tier route; `[env] require_native_apple_evidence = true` makes this a \
         refusal instead of a warning",
        project_root.display(),
        APPLE_ONLY_MODULES.len()
    ))
}

/// The provider's own directory inside the project:
/// `<project>/.devcroft/swift/{bin,cache,tmp}`.
///
/// Under the artifact directory devcroft already owns and every sample
/// ignores in git, not under SwiftPM's `.build`: the project can wipe
/// `.build` from inside the sandbox (`swift package reset` does) and the
/// sandbox must survive that. Not keyed by sandbox name the way
/// `services::artifact_dir` is, because nothing here is per-sandbox:
/// two sandboxes on one project root would share a `.build` too.
pub struct ProviderDirs {
    project_root: PathBuf,
    /// Holds the `swift` shim; first on the sandbox's `PATH`.
    bin: PathBuf,
    /// SwiftPM's package cache, clang's module cache, xcrun's lookup db.
    cache: PathBuf,
    /// `TMPDIR`.
    tmp: PathBuf,
}

impl ProviderDirs {
    pub fn create(project_root: &Path) -> Result<Self, ProviderError> {
        let root = project_root
            .join(super::super::services::ARTIFACT_DIR)
            .join("swift");
        let dirs = ProviderDirs {
            project_root: project_root.to_path_buf(),
            bin: root.join("bin"),
            cache: root.join("cache"),
            tmp: root.join("tmp"),
        };
        for d in [&dirs.bin, &dirs.cache, &dirs.tmp] {
            std::fs::create_dir_all(d).map_err(|e| {
                ProviderError::ResolutionFailed(format!("creating {}: {e}", d.display()))
            })?;
        }
        Ok(dirs)
    }

    /// Where the shim lives; `up` names it so the wrapper is never a
    /// surprise found by `which swift`.
    pub fn shim_path(project_root: &Path) -> PathBuf {
        project_root
            .join(super::super::services::ARTIFACT_DIR)
            .join("swift")
            .join("bin")
            .join("swift")
    }

    /// Write the `swift` wrapper. Rewritten on every resolution so a
    /// toolchain change (`xcode-select -s`) is reflected at the next `up`.
    ///
    /// **Why a wrapper at all, and why this small.** Two SwiftPM behaviours
    /// have no environment lever (`activated_env` lists the ones that do):
    ///
    /// - SwiftPM applies its *own* Seatbelt profile (`sandbox-exec`) when
    ///   it compiles `Package.swift` and runs plugins. Seatbelt does not
    ///   nest — a second `sandbox_init` from an already sandboxed process
    ///   returns `EPERM`, measured — so inside a devcroft sandbox every
    ///   `swift build` died with `Invalid manifest`. `--disable-sandbox`
    ///   turns SwiftPM's off; devcroft's, the outer and stricter one, is
    ///   what remains. Nothing is weakened: SwiftPM's sandbox permitted
    ///   reads of the whole home directory, devcroft's does not.
    /// - SwiftPM's package cache defaults to `~/Library/Caches/org.swift.swiftpm`
    ///   with `~` taken from the password database, not `$HOME`, so the
    ///   sandbox's own home does not redirect it. `--cache-path` does.
    ///
    /// The shim adds exactly those two flags, to exactly the subcommands
    /// that accept them, and execs the real binary for everything else.
    /// It is a shell script, not a binary, so `policy --render`'s reader
    /// can open it and see the whole of what it does; the host `/bin/sh`
    /// it needs is already the artifact tier's declared shell grant.
    fn write_shim(&self, real_swift: &Path) -> Result<(), ProviderError> {
        use std::os::unix::fs::PermissionsExt;
        let shim = self.bin.join("swift");
        let script = format!(
            "#!/bin/sh\n\
             # devcroft's swift shim — written by the swift provider at every `up`.\n\
             # Adds the two SwiftPM flags that have no environment equivalent:\n\
             #   --disable-sandbox  SwiftPM's own Seatbelt cannot nest inside devcroft's\n\
             #   --cache-path       SwiftPM's package cache, otherwise ~/Library/Caches\n\
             # Everything else is passed to the toolchain's swift unchanged.\n\
             real={real}\n\
             cache={cache}\n\
             case \"$1\" in\n\
               build|run|test|package)\n\
                 sub=$1; shift\n\
                 exec \"$real\" \"$sub\" --disable-sandbox --cache-path \"$cache\" \"$@\" ;;\n\
               *)\n\
                 exec \"$real\" \"$@\" ;;\n\
             esac\n",
            real = shell_quote(&real_swift.to_string_lossy()),
            cache = shell_quote(&self.cache.join("swiftpm").to_string_lossy()),
        );
        std::fs::write(&shim, script).map_err(|e| {
            ProviderError::ResolutionFailed(format!("writing {}: {e}", shim.display()))
        })?;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| ProviderError::ResolutionFailed(format!("chmod {}: {e}", shim.display())))
    }
}

/// Single-quote `s` for `sh`, escaping embedded single quotes.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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

    /// Frameworks are read out of `Package.swift` as text, in both
    /// spellings that appear in real manifests. The point is not that
    /// text parsing is better than `dump-package` — it is worse — but
    /// that `dump-package` costs the provider its criterion-4 position,
    /// and this signal is not worth that.
    #[test]
    fn frameworks_are_read_from_the_manifest_without_evaluating_it() {
        let text = r#"
            targets: [.executableTarget(name: "a",
                linkerSettings: [.linkedFramework("AppKit"),
                                 .unsafeFlags(["-framework", "Security"])])]
        "#;
        let mut got = frameworks_named_in_manifest(text);
        got.sort();
        assert_eq!(got, vec!["AppKit", "Security"]);
    }

    #[test]
    fn a_manifest_naming_no_framework_yields_none() {
        assert!(frameworks_named_in_manifest("let package = Package(name: \"x\")").is_empty());
    }

    /// A truncated manifest must not panic the scanner — it is reading
    /// arbitrary project text, so malformed input is expected input.
    #[test]
    fn a_truncated_framework_call_does_not_panic() {
        for text in [
            ".linkedFramework(",
            ".linkedFramework(\"",
            "\"-framework\"",
            "\"-framework\", \"",
        ] {
            let _ = frameworks_named_in_manifest(text);
        }
    }

    /// **The provider is macOS-only, and fails closed elsewhere.** Swift
    /// exists on Linux; an Xcode-backed provider does not, so resolving
    /// silently under the same name would make one manifest mean two
    /// different guarantees on two machines.
    #[test]
    fn the_provider_is_refused_off_macos() {
        let result = ensure_macos_host();
        if cfg!(target_os = "macos") {
            assert!(result.is_ok(), "must resolve on its own platform");
        } else {
            match result {
                Err(ProviderError::CoveredByQualifiedProvider { provider, reason }) => {
                    assert_eq!(provider, "swift");
                    assert!(reason.contains("macOS"));
                    assert!(reason.contains("nix") || reason.contains("flox"));
                }
                other => panic!("expected a platform refusal, got {other:?}"),
            }
        }
    }

    /// Every cache and scratch lever points inside the project, and the
    /// shim's directory leads `PATH` — the whole reason a build needs no
    /// grant outside the project root. Asserted on the environment, not
    /// on a build, so it runs on every host.
    #[test]
    fn every_lever_points_inside_the_project_and_the_shim_leads_path() {
        let baseline = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);
        let toolchain = parse_target_info(REAL_TARGET_INFO.as_bytes()).unwrap();
        let root = std::env::temp_dir().join(format!("devcroft-swift-dirs-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let dirs = ProviderDirs::create(&root).unwrap();
        let env = activated_env(
            &baseline,
            Path::new("/usr/bin/swift"),
            &toolchain,
            Some(&dirs),
        );
        let inside = |key: &str| {
            let v = env.get(key).unwrap_or_else(|| panic!("{key} must be set"));
            assert!(
                Path::new(v).starts_with(&root),
                "{key}={v} must be inside the project root {}",
                root.display()
            );
        };
        for key in [
            "SWIFTPM_BUILD_DIR",
            "TMPDIR",
            "CLANG_MODULE_CACHE_PATH",
            "xcrun_db",
        ] {
            inside(key);
        }
        assert_eq!(
            env.get("SWIFTPM_BUILD_DIR").unwrap(),
            &root.join(".build").to_string_lossy()
        );
        let path = env.get("PATH").unwrap();
        assert!(
            path.starts_with(&format!("{}:", dirs.bin.display())),
            "the shim's directory must lead PATH, got {path}"
        );
        assert!(
            path.contains(":/usr/bin:"),
            "the toolchain's bin must follow, got {path}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The shim is a readable script that adds exactly the two flags with
    /// no environment lever, to exactly the subcommands that take them.
    #[test]
    fn the_shim_adds_the_two_flags_only_to_swiftpm_subcommands() {
        let root = std::env::temp_dir().join(format!("devcroft-swift-shim-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let dirs = ProviderDirs::create(&root).unwrap();
        dirs.write_shim(Path::new("/tool/bin/swift")).unwrap();
        let shim = ProviderDirs::shim_path(&root);
        let text = std::fs::read_to_string(&shim).unwrap();
        assert!(text.starts_with("#!/bin/sh\n"));
        assert!(text.contains("build|run|test|package)"));
        assert!(text.contains("--disable-sandbox --cache-path"));
        assert!(text.contains("real='/tool/bin/swift'"));
        // The pass-through arm execs the real binary with the arguments
        // untouched, so `swiftc`-style invocations are never rewritten.
        assert!(text.contains("exec \"$real\" \"$@\""));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&shim).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let _ = std::fs::remove_dir_all(&root);
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
        match apple_evidence_advice(&dir) {
            Some(reason) => {
                assert!(reason.contains("nix") && reason.contains("flox"));
                assert!(
                    reason.contains("platforms"),
                    "the refusal must pre-empt the obvious objection — that the package \
                     declares a macOS platform — since that is not evidence; got: {reason}"
                );
            }
            None => panic!("a portable package must draw the advice"),
        }

        // A framework named in Package.swift alone is conclusive, with
        // no Apple import anywhere in the sources.
        std::fs::write(
            dir.join("Package.swift"),
            "// swift-tools-version:5.9\nlinkerSettings: [.linkedFramework(\"AppKit\")]\n",
        )
        .unwrap();
        assert!(apple_evidence_advice(&dir).is_none());
        std::fs::remove_file(dir.join("Package.swift")).unwrap();

        // An unguarded Apple import alone is conclusive, with no
        // framework.
        std::fs::write(
            dir.join("Sources/app/main.swift"),
            "import AppKit\nprint(1)\n",
        )
        .unwrap();
        assert!(apple_evidence_advice(&dir).is_none());

        // Guarded again: back to refused, so the guard test is live
        // through the real file scan and not only in `imports()`.
        std::fs::write(
            dir.join("Sources/app/main.swift"),
            "#if canImport(AppKit)\nimport AppKit\n#endif\n",
        )
        .unwrap();
        assert!(
            apple_evidence_advice(&dir).is_some(),
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
            scan_apple_evidence(&dir).is_empty(),
            "a dependency's AppKit import must not qualify this package"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The deliverable, not the source.** A Mac app whose Swift is
    /// entirely Foundation still cannot be produced by a Linux closure —
    /// bundle, entitlements, code signature and `xcodebuild` are all
    /// Apple-side. Before this, such a project was refused and sent to
    /// flox, which can do none of those: a wrong refusal with no remedy.
    #[test]
    fn an_apple_project_artifact_is_evidence_even_with_portable_sources() {
        for artifact in [
            "Info.plist",
            "App.entitlements",
            "Assets.xcassets",
            "Main.storyboard",
        ] {
            let dir = std::env::temp_dir().join(format!(
                "devcroft-swift-artifact-{}-{}-{}",
                std::process::id(),
                line!(),
                artifact.replace('.', "_")
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("Sources/app")).unwrap();
            std::fs::write(
                dir.join("Sources/app/main.swift"),
                "import Foundation\nprint(1)\n",
            )
            .unwrap();

            // Portable sources alone: refused.
            assert!(
                apple_evidence_advice(&dir).is_some(),
                "control: Foundation-only with no artifact must be refused"
            );

            std::fs::write(dir.join(artifact), b"x").unwrap();
            assert!(
                apple_evidence_advice(&dir).is_none(),
                "{artifact} must count as evidence"
            );

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// An `.xcodeproj` is a directory, so it has to be recognised as one
    /// rather than only as a file name.
    #[test]
    fn an_xcodeproj_directory_is_evidence() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-swift-xcodeproj-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("App.xcodeproj")).unwrap();
        std::fs::create_dir_all(dir.join("Sources/app")).unwrap();
        std::fs::write(dir.join("Sources/app/main.swift"), "import Foundation\n").unwrap();

        let evidence = scan_apple_evidence(&dir);
        assert!(
            evidence
                .artifacts
                .iter()
                .any(|a| a.ends_with("App.xcodeproj")),
            "got {:?}",
            evidence.artifacts
        );
        assert!(apple_evidence_advice(&dir).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A dependency's `Info.plist` is no more evidence about this package
    /// than a dependency's import was — `.build` stays excluded for both
    /// halves of the evidence, not just the imports half.
    #[test]
    fn an_artifact_under_build_is_not_evidence() {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-swift-artifactdep-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".build/checkouts/other")).unwrap();
        std::fs::create_dir_all(dir.join("Sources/app")).unwrap();
        std::fs::write(dir.join("Sources/app/main.swift"), "import Foundation\n").unwrap();
        std::fs::write(dir.join(".build/checkouts/other/Info.plist"), b"x").unwrap();

        assert!(
            scan_apple_evidence(&dir).is_empty(),
            "a dependency's Info.plist must not qualify this package"
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

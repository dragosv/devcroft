# env-provider Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: Guarantee tier is a provider-supplied value
The system SHALL model each provider's guarantee tier as a value the
provider itself supplies — `closure` or `artifact` — rather than
inferring it from the shape of its grants. Every provider, including one
injected through the test seam, SHALL answer it.

#### Scenario: Closure providers report closure
- **WHEN** the tier of `flox`, `nix`, or `devbox` is requested
- **THEN** each reports `closure`

#### Scenario: The swift provider reports artifact
- **WHEN** the tier of `swift` is requested
- **THEN** it reports `artifact`

### Requirement: swift provider resolution
The system SHALL resolve a Swift environment by discovering the host
toolchain's paths from `swift -print-target-info`, which returns
structured data and evaluates no project file, and injecting the
resulting environment diff into the keeper.

Resolution SHALL respect `Package.resolved` and SHALL NOT update it,
resolve dependency versions, or contact a package index.

#### Scenario: Toolchain paths are discovered as data
- **WHEN** resolution runs on a host with a working Swift toolchain
- **THEN** the runtime library paths and runtime resource path reported
  by `swift -print-target-info` are granted read-only with origin
  `provider:swift`

#### Scenario: Missing toolchain fails at layer provider
- **WHEN** `swift` is not on `PATH`
- **THEN** `up` fails at layer `provider` naming the missing binary,
  before any restriction is applied

#### Scenario: Missing package manifest fails with a remedy
- **WHEN** the project has no `Package.swift`
- **THEN** `up` fails at layer `provider` naming `swift package init` as
  the fix, distinct from a missing-toolchain failure

### Requirement: swift resolution opens no project file
The system SHALL resolve a `swift` environment from the host toolchain
alone — `xcode-select`, `xcrun`, `swift -print-target-info` — and SHALL
NOT read, open or evaluate `Package.swift` or any other project file
during resolution.

`Package.swift` is a Swift program that SwiftPM compiles and executes to
produce the package description, and SwiftPM's own sandbox around that
evaluation permits reads and exec. Dependency resolution therefore belongs
inside the sandbox, at build time, under the policy the project declared.

#### Scenario: No execution disclosure is recorded
- **WHEN** a `swift` environment is resolved
- **THEN** the resolution does not record that project code ran, and `up`
  prints no execution warning

#### Scenario: Resolution succeeds without a readable package graph
- **WHEN** `Package.swift` declares dependencies with no `Package.resolved`
- **THEN** resolution still succeeds, because devcroft materializes no
  dependencies host-side

### Requirement: swift runs only on macOS
The system SHALL refuse `env.provider = "swift"` on any platform other
than macOS, at layer `provider`, naming the platform. The provider
resolves an Xcode or Command Line Tools toolchain; resolving a different
toolchain under the same provider name would make one manifest mean two
different guarantees on two machines.

#### Scenario: Refused off macOS
- **WHEN** a manifest declares `provider = "swift"` on Linux
- **THEN** `up` fails at layer `provider` naming macOS and pointing at a
  closure provider

### Requirement: swift advises where a qualifying provider would serve the project
The system SHALL honour an explicit `env.provider = "swift"` and SHALL,
at `up`, print exactly one warning when the project shows no dependency
on Apple platforms — because a closure-tier provider would serve such a
package and serve it better, reproducibly and without executing
`Package.swift` on the host. The warning SHALL name the providers that
would serve the project, SHALL state what evidence was searched for, and
SHALL name the opt-in below. The system SHALL refuse instead of warning
when the manifest sets `[env] require_native_apple_evidence = true`, at
layer `provider`, before any restriction is applied.

The scan is advice and not a gate because it cannot prove a closure
provider can build the project, can miss an Apple-native project with
an unusual layout, can accept one on a residual artifact, and would
turn a declared choice into an inference that goes false as the project
evolves. A project that wants the inference to bind says so in its
committed manifest.

Evidence is of three kinds: a linked Apple framework (however spelled,
including `-framework` passed through unsafe linker flags); an `import`
of an Apple-only module that is not inside a conditional-compilation
guard; or an Apple project artifact such as `Info.plist`, an
entitlements file, an `.xcodeproj`, or an asset catalog. The third kind
concerns the *deliverable* rather than the source. A declared
`platforms:` entry SHALL NOT be treated as evidence, since it constrains
only Apple platform minimums and is ignored on Linux. Evidence under
`.build/` is a dependency's, not this project's, and SHALL NOT count.

#### Scenario: A portable package comes up with one warning
- **WHEN** a package imports only modules available on Linux and the
  manifest does not set the opt-in
- **THEN** `up` succeeds and prints exactly one warning naming `nix`,
  `flox`, what was searched for, and `require_native_apple_evidence`

#### Scenario: A portable package is refused on request
- **WHEN** the same package's manifest sets
  `[env] require_native_apple_evidence = true`
- **THEN** `up` fails at layer `provider` with exit code 3, naming the
  key that asked for the refusal

#### Scenario: Evidence silences the advice
- **WHEN** a target declares a linked Apple framework, or a source file
  imports an Apple-only module at top level, or the project holds an
  Apple project artifact
- **THEN** `up` prints no such warning

#### Scenario: A guarded Apple-only import is not evidence
- **WHEN** the only Apple-only import is inside `#if canImport(...)`
- **THEN** the package is treated as portable

#### Scenario: The opt-in under another provider is a config error
- **WHEN** a manifest sets `require_native_apple_evidence = true` with
  any provider but `swift`
- **THEN** parsing fails at layer `config`, naming the key and the
  provider it applies to

### Requirement: The build runs through a provider-owned shim
The system SHALL place a `swift` wrapper ahead of the toolchain's `swift`
on the sandbox's `PATH`, at `<project>/.devcroft/swift/bin/swift`,
rewritten at every resolution, which adds `--disable-sandbox` and
`--cache-path <project>/.devcroft/swift/cache/swiftpm` to the `build`,
`run`, `test` and `package` subcommands and passes every other
invocation to the toolchain unchanged. `up` SHALL say the shim exists,
where it is, and what it adds. The shim SHALL be a readable shell script.

SwiftPM applies its own Seatbelt profile to manifest compilation and
plugins, and Seatbelt does not nest: inside a devcroft sandbox every
build failed at `Invalid manifest` until the inner sandbox was turned
off. devcroft's sandbox — the outer one, which unlike SwiftPM's permits
no read of the home directory — is what remains. SwiftPM's package cache
resolves `~` from the password database, so the sandbox's own `HOME`
does not move it; `--cache-path` does. Neither flag has an environment
equivalent.

#### Scenario: A plain build succeeds with only the project granted
- **WHEN** a manifest grants only the project root and a session runs
  `swift build` then `swift run`
- **THEN** both succeed, the products land in `.build/`, every cache the
  toolchain writes lands under `.devcroft/swift/`, and no `sandbox_apply`
  or "couldn't create cache file" diagnostic appears

#### Scenario: The shim is what `swift` resolves to
- **WHEN** a session runs `command -v swift`
- **THEN** the answer is the shim's path, and `up`'s output named it

### Requirement: Every cache and scratch lever points inside the project
The system SHALL set, in the resolved environment, every variable the
toolchain honours for where it writes — `SWIFTPM_BUILD_DIR`, `TMPDIR`,
`CLANG_MODULE_CACHE_PATH`, and `xcrun_db` — to paths inside the project,
under `.build/` or `.devcroft/swift/`, so that a build needs no write
grant outside the project root. The provider's own directory SHALL be
under `.devcroft/`, not `.build/`, so a `swift package reset` from inside
does not remove the sandbox's temporary directory.

#### Scenario: No write grant outside the project is needed
- **WHEN** a build runs with the project root as the only manifest grant
- **THEN** it succeeds, and nothing is written under the Darwin per-user
  temporary or cache directories

### Requirement: The toolchain's host dependencies are the provider's grants
The system SHALL grant, read-only and with origin `provider:swift`,
what the toolchain and Foundation read from the host beyond the toolchain
itself, each because a build or a program failed without it: under
Xcode, the app bundle's `Contents` (the toolchain, `xcodebuild`'s
frameworks and plug-ins), the license record
`/Library/Preferences/com.apple.dt.Xcode.plist`, `/Library/Apple`
(device-support frameworks behind a `/System` symlink), and
`/usr/share/firmlinks` (FSEvents' firmlink table, without which
`xcodebuild` crashes); on every toolchain, `/usr/share/icu` and
`/var/db/timezone` (Foundation's formatting data — empty output without
them, silently), `/var/select` (macOS's `/bin/sh` selector), and the
host's userland `/usr/bin` and `/bin`. The `swift` resolved SHALL be the
toolchain's own binary (`xcrun --find swift`), not the `/usr/bin`
trampoline, so the grants name the toolchain rather than a shim's parent.

The host userland is granted deliberately: this tier has no closure to
supply `grep`, `sed`, `env` or `python3`, and its definition — runtime
linked against the host — is the same fact one directory over. With
execute following the read grant, every host binary in `/usr/bin` runs
inside a swift sandbox, and that is the tier's stated cost, not a gap.

#### Scenario: The grants are visible and attributable
- **WHEN** `policy --render` runs for a swift sandbox
- **THEN** each of the paths above that exists on the host appears
  under `filesystem.read` with origin `provider:swift`

#### Scenario: A grant names only what exists
- **WHEN** a path above does not exist on this host
- **THEN** it is not granted, and nothing else changes — the grants are
  existence-checked at resolution, never emitted for absent paths

### Requirement: swift declares no services
The system SHALL report the `swift` provider as having no service
mechanism, so that a project declaring services under it fails loudly
rather than silently starting nothing.

#### Scenario: Services declared under swift are refused
- **WHEN** a manifest names provider `swift` and declares `[services]`
- **THEN** `up` fails naming the provider as unable to supply services

### Requirement: swift staleness
The system SHALL detect a changed Swift environment by fingerprinting
`Package.swift` together with `Package.resolved`, keeping an absent
lockfile distinct from a present-but-empty one.

#### Scenario: Editing the package manifest is stale
- **WHEN** `Package.swift` changes after `up`
- **THEN** `status` reports the environment stale

#### Scenario: A lockfile appearing is itself a change
- **WHEN** `Package.resolved` is created after `up`
- **THEN** `status` reports the environment stale

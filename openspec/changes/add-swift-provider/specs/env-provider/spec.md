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

### Requirement: swift is refused where a qualifying provider serves the project
The system SHALL refuse `env.provider = "swift"` for a package that shows
no dependency on Apple platforms, because a closure-tier provider serves
such a package and serves it better — reproducibly, without executing
`Package.swift` on the host.

Acceptance SHALL require positive evidence of one of three kinds: a
linked Apple framework (however spelled, including `-framework` passed
through unsafe linker flags); an `import` of an Apple-only module that is
not inside a conditional-compilation guard; or an Apple project artifact
such as `Info.plist`, an entitlements file, an `.xcodeproj`, or an asset
catalog.

The third kind concerns the *deliverable* rather than the source: a
project whose Swift is portable may still be impossible for a closure to
produce, because app bundles, entitlements and code signing are
Apple-side.

A declared `platforms:` entry SHALL NOT be treated as evidence, since it
constrains only Apple platform minimums and is ignored on Linux.

The refusal SHALL name the providers that serve the project instead, and
SHALL state what evidence was searched for.

#### Scenario: A portable package is refused
- **WHEN** a package imports only modules available on Linux
- **THEN** `up` fails at layer `provider` naming `nix` and `flox`

#### Scenario: A linked Apple framework is accepted
- **WHEN** a target declares a linked Apple framework
- **THEN** resolution proceeds

#### Scenario: An unguarded Apple-only import is accepted
- **WHEN** a source file imports an Apple-only module at top level
- **THEN** resolution proceeds

#### Scenario: A guarded Apple-only import is not evidence
- **WHEN** the only Apple-only import is inside `#if canImport(...)`
- **THEN** the package is treated as portable and refused

#### Scenario: An Apple project artifact is accepted with portable sources
- **WHEN** the sources import only modules available on Linux but the
  project holds an `Info.plist`, entitlements file, `.xcodeproj`, or asset
  catalog
- **THEN** resolution proceeds

#### Scenario: A dependency's evidence is not this project's evidence
- **WHEN** an Apple-only import or an Apple project artifact appears only
  under `.build/`
- **THEN** the package is treated as portable and refused

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

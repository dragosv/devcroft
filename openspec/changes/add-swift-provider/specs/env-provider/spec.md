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

### Requirement: swift resolution discloses that it executes project code
The system SHALL report that resolving a `swift` environment executed the
project's own code, because `Package.swift` is a program that SwiftPM
compiles and runs to produce the package description, and no entry point
exists that returns the package graph without doing so.

The report SHALL be unconditional for this provider and SHALL NOT depend
on inspection of the manifest's contents.

#### Scenario: The disclosure fires on every swift resolution
- **WHEN** a `swift` environment is resolved
- **THEN** the resolution records that project code ran during
  provisioning, and `up` prints the warning naming the provider

#### Scenario: Other providers are unaffected
- **WHEN** a `flox`, `nix`, or `devbox` environment is resolved
- **THEN** no such disclosure is recorded, exactly as before this change

### Requirement: swift lockfile precondition
The system SHALL require `Package.resolved` when, and only when, the
package declares external dependencies, since SwiftPM does not create the
file for a package that has none.

#### Scenario: Dependencies without a lockfile are refused
- **WHEN** `Package.swift` declares at least one dependency and
  `Package.resolved` is absent
- **THEN** `up` fails at layer `provider` naming `swift package resolve`
  as the fix

#### Scenario: A dependency-free package needs no lockfile
- **WHEN** `Package.swift` declares no dependencies and
  `Package.resolved` is absent
- **THEN** resolution proceeds

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

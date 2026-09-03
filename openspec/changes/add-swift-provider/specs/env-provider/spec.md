# env-provider Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: Swift provider resolution is toolchain-only

The system SHALL resolve a Swift environment by discovering the host's
selected Xcode or Command Line Tools toolchain, and SHALL NOT read, open,
parse or evaluate any file belonging to the project in order to do so.

Resolution SHALL produce the environment diff from toolchain discovery
alone: `DEVELOPER_DIR` from the selected developer directory, `SDKROOT`
from the macOS SDK, and the toolchain's `usr/bin` prepended to a `PATH`
built on the same fixed pre-activation baseline the flox, nix and devbox
providers use.

This is the whole difference between this provider and the other three,
and it SHALL NOT be relaxed for convenience: SwiftPM offers no entry point
that returns a resolved package graph without compiling and executing
`Package.swift`, so any manifest-reading resolution would execute project
code host-side, before any restriction exists.

#### Scenario: A Swift project resolves without its manifest being read

- **WHEN** `up` runs in a project whose `Package.swift` has a side effect
  that would be observable if the file were executed
- **THEN** resolution SHALL succeed
- **AND** the side effect SHALL NOT be observed
- **AND** the captured environment SHALL contain `DEVELOPER_DIR` and
  `SDKROOT` pointing inside the selected toolchain

#### Scenario: A project with no Package.swift still resolves

- **WHEN** `up` runs with `env.provider = "swift"` in a project root
  containing no `Package.swift`
- **THEN** resolution SHALL succeed, because the provider resolves a
  toolchain rather than a package graph
- **AND** the guarantee-tier notice SHALL still be emitted

### Requirement: Dependency resolution happens inside the boundary

The system SHALL NOT fetch, resolve, check out or build SwiftPM
dependencies during the provisioning phase.

Dependency work SHALL happen inside the sandbox, under the policy the
project declared, where a `Package.swift` that executes is confined by
that policy rather than running with the invoking user's own access.

Where a project's dependencies require network access to fetch, that
access SHALL be obtained through the manifest's `[network]` allowlist like
any other runtime network use, and SHALL NOT be granted implicitly by the
provider.

#### Scenario: A manifest with dependencies is not resolved at `up`

- **WHEN** `up` runs in a project whose `Package.swift` declares remote
  package dependencies and whose `.build` directory does not exist
- **THEN** `up` SHALL NOT create `.build/checkouts`
- **AND** `up` SHALL NOT contact any package host

#### Scenario: Fetching inside the sandbox without an allowlist entry is refused

- **WHEN** a session runs `swift build` inside the sandbox, the project
  declares remote dependencies, and no `[network]` entry permits the
  package host
- **THEN** the fetch SHALL fail
- **AND** the failure SHALL be attributable to the declared policy rather
  than presented as a provider error

### Requirement: The Swift provider is macOS-only and fails closed elsewhere

The system SHALL accept `env.provider = "swift"` only on macOS. On any
other platform `up` SHALL fail at layer `provider` with exit code 3,
naming the platform and stating that the provider is backed by Xcode or
the Command Line Tools.

The system SHALL NOT fall back to a Linux Swift toolchain under the same
provider name. Swift exists on Linux; an Xcode-backed environment does
not, and resolving a materially different toolchain under one provider
name would make the provider name mean two things.

#### Scenario: Declaring the Swift provider on Linux

- **WHEN** `up` runs on Linux with `env.provider = "swift"`
- **THEN** it SHALL fail at layer `provider` with exit code 3
- **AND** the message SHALL name the platform and the backing toolchain
- **AND** it SHALL NOT report the provider as unknown or unsupported

### Requirement: Swift preconditions are verified before resolution

The system SHALL verify, cheaply and before any environment capture, that
a developer directory is selected and usable, that a macOS SDK is present
within it, and that the Swift compiler can be located through it.

Verification SHALL probe by *executing* the toolchain rather than by
testing for a path, matching the capability-not-presence rule the test
suite already follows. A selected developer directory that no longer
exists, and an unaccepted licence agreement, both leave the path in place
while making the toolchain unusable.

Where verification fails, `up` SHALL fail at layer `provider` with exit
code 3, naming which precondition failed and the command that fixes it.

#### Scenario: The selected developer directory is missing

- **WHEN** the selected developer directory does not exist or cannot
  produce an SDK path
- **THEN** `up` SHALL fail at layer `provider` with exit code 3
- **AND** the message SHALL name `xcode-select` as the way to correct it

#### Scenario: The licence has not been accepted

- **WHEN** the toolchain is installed but refuses to run pending licence
  acceptance
- **THEN** `up` SHALL fail at layer `provider` with exit code 3
- **AND** the message SHALL distinguish this from a missing toolchain

### Requirement: The Swift provider declares its host library grants

The system SHALL compile the host paths a Swift-built artifact needs at
runtime as provider grants with a `provider:swift` origin, and SHALL NOT
rely on the baseline for them.

The grants SHALL name the developer directory and the dynamic linker's
shared cache. They SHALL NOT name individual system dylib paths such as
`/usr/lib/libSystem.B.dylib`: those paths do not exist as files on a
supported macOS, so granting them grants nothing while appearing to.

Provider resolution SHALL NOT widen the policy beyond these declared
grants. Where the toolchain would require write access outside the project
root, `up` SHALL fail naming the path rather than granting it.

#### Scenario: Granted paths are attributed and renderable

- **WHEN** `policy --render` runs for a resolved Swift sandbox
- **THEN** every host path the provider contributed SHALL appear with
  origin `provider:swift`
- **AND** nothing reaching the backend SHALL be absent from that output

#### Scenario: A binary built in the sandbox runs in the sandbox

- **WHEN** a session builds an executable from a SwiftPM project inside
  the sandbox and then runs it
- **THEN** it SHALL execute successfully
- **AND** it SHALL do so without any grant outside the project root, the
  declared `provider:swift` paths, and the baseline

### Requirement: SwiftPM's home-directory caches are redirected, not granted

The system SHALL direct SwiftPM's cache, scratch and configuration state
into the project root rather than granting write access to the invoking
user's home directory.

SwiftPM writes to three separate home-directory locations. Granting them
would give project code write access to the user's home directory for the
lifetime of the sandbox, which the baseline denies by design; the provider
SHALL use SwiftPM's own path overrides instead.

#### Scenario: A build writes no state into the home directory

- **WHEN** a session runs a full `swift build` inside the sandbox
- **THEN** the build SHALL succeed
- **AND** no file SHALL be created or modified under the invoking user's
  home directory
- **AND** the policy SHALL contain no write grant for that directory

### Requirement: Providers whose activation is not capturable report it

The system SHALL report, rather than silently accept, a provider whose
environment cannot be obtained without executing project code.

For the Swift provider this requirement is satisfied by construction — the
provider never asks SwiftPM for a package graph — and the requirement is
stated so that a future change adding manifest-derived resolution cannot
do so without confronting it.

#### Scenario: Manifest-derived resolution is added later

- **WHEN** a change introduces resolution that requires evaluating
  `Package.swift` host-side
- **THEN** it SHALL either run that evaluation inside the boundary, or
  fail closed at layer `provider`
- **AND** it SHALL NOT execute the manifest host-side without reporting it

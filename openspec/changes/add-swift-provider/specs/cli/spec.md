# cli Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: The guarantee tier is stated once at `up` and shown in `status`

The system SHALL state the resolved provider's guarantee tier exactly once
at `up`, and SHALL show it in `status` for the lifetime of the sandbox.

For an artifact-tier provider the notice SHALL say what the user is *not*
getting: that identical downloaded artifacts are guaranteed, that runtime
behaviour depends on host libraries, and that environments are not shared
between sandboxes the way a content-addressed store shares them. Naming
only the tier would be marketing two guarantees under one word, which the
standing framing rule forbids.

The notice SHALL be emitted once per `up`, in the same shape as the
existing degraded-capability warning, and SHALL NOT be repeated by
`status`.

#### Scenario: Bringing up an artifact-tier sandbox

- **WHEN** `up` succeeds with `env.provider = "swift"`
- **THEN** exactly one tier notice SHALL be printed
- **AND** it SHALL name the tier and the property that does not hold

#### Scenario: Bringing up a closure-tier sandbox

- **WHEN** `up` succeeds with `env.provider = "flox"`, `"nix"` or
  `"devbox"`
- **THEN** no artifact-tier notice SHALL be printed

#### Scenario: `status` does not repeat the notice

- **WHEN** `status` runs against a running Swift sandbox
- **THEN** it SHALL report the tier as part of the sandbox's state
- **AND** it SHALL NOT re-emit the `up` notice

### Requirement: `doctor` reports Swift toolchain preconditions

The system SHALL add a Swift arm to `doctor`, reporting whether a
developer directory is selected, which installation backs it, the SDK
version it provides, and whether the toolchain runs.

`doctor` SHALL probe by executing the toolchain, not by testing for a
path. A stale `xcode-select` selection and an unaccepted licence both
leave a plausible path in place while making every build fail, and both
are the kind of failure a user would otherwise attribute to devcroft.

On a non-macOS host the Swift arm SHALL report the provider as
unavailable on this platform rather than as broken.

#### Scenario: A healthy macOS toolchain

- **WHEN** `doctor` runs on macOS with a working Command Line Tools or
  Xcode installation
- **THEN** it SHALL report the selected developer directory, which of the
  two backs it, and the SDK version

#### Scenario: A selected directory that no longer exists

- **WHEN** `doctor` runs with a developer directory selected but absent
- **THEN** it SHALL report the precondition as failing
- **AND** it SHALL name the command that corrects the selection

#### Scenario: `doctor` on Linux

- **WHEN** `doctor` runs on Linux and a discoverable manifest declares
  `env.provider = "swift"`
- **THEN** it SHALL report the provider as unavailable on this platform
- **AND** it SHALL NOT report a missing-toolchain failure

### Requirement: Help text lists no new command

The system SHALL implement this change without adding to the MVP command
surface.

The Swift provider is a value of an existing manifest key, not a new verb.
Where a new command would be needed to expose it, that is evidence the
provider seam did not generalize and SHALL be treated as a design problem
rather than resolved by adding a command.

#### Scenario: The command surface is unchanged

- **WHEN** this change is complete
- **THEN** `devcroft --help` SHALL list the same commands as before
- **AND** the help-and-version test SHALL pass unmodified

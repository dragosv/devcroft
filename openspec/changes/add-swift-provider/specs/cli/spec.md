# cli Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: Guarantee tier is user-visible
The system SHALL print the resolved environment's guarantee tier once at
`up` and on every `status`, so that two sandboxes backed by different
guarantees can be told apart without reading documentation.

For the `artifact` tier the line SHALL name what the tier does not
promise — that the runtime links against host libraries — rather than
printing the bare word.

#### Scenario: status names the tier
- **WHEN** `devcroft status` runs for a sandbox
- **THEN** the output contains a line naming the provider's guarantee
  tier

#### Scenario: artifact tier is qualified, not merely labelled
- **WHEN** the provider is `swift`
- **THEN** the tier line states that the runtime is host-linked

#### Scenario: closure sandboxes are unchanged in substance
- **WHEN** the provider is `flox`, `nix`, or `devbox`
- **THEN** the tier line reports `closure`

## MODIFIED Requirements

### Requirement: init
The system SHALL provide `devcroft init` which detects an existing flox
environment (`.flox/`), an existing devbox project (`devbox.json`), an
existing nix flake (`flake.nix`), a SwiftPM package (`Package.swift`),
single-ecosystem toolchain pins (`rust-toolchain.toml`, `.nvmrc`,
`.python-version`), and the project language, generates a minimal
`devcroft.toml` with commented-out common options, and never overwrites an
existing manifest without `--force`.

Where several environments are present, detection SHALL apply a fixed,
documented order — flox, then devbox, then a bare flake, then a SwiftPM
package — and SHALL state in one line which others were found and remain
available.

`swift` SHALL rank below every closure-tier provider, because it is the
only provider that fails the qualification test in `docs/decisions.md` §1
and a project holding any closure environment SHALL keep the stronger
guarantee.

`init` SHALL select `swift` only for a project showing filesystem evidence
that it needs Apple platforms — an unguarded Apple-only import, or an Apple
project artifact — so that it never generates a manifest the provider would
refuse at `up`. Detection SHALL NOT execute the package's own code.

Where `swift` is selected, `init` SHALL name both of the tier's costs at
the point of selection: that the toolchain comes from the host, and that
resolving the environment executes `Package.swift` on the host at every
`up`.

#### Scenario: A SwiftPM package selects the swift provider
- **WHEN** `init` runs in a directory holding `Package.swift` and no flox,
  devbox, or flake environment
- **THEN** the generated manifest declares `provider = "swift"`

#### Scenario: Every closure provider outranks a SwiftPM package
- **WHEN** `init` runs in a directory holding `Package.swift` alongside a
  `.flox/`, a `devbox.json`, or a `flake.nix`
- **THEN** the generated manifest declares that closure provider, not
  `swift`

#### Scenario: Selecting swift discloses what it costs
- **WHEN** `init` selects `swift`
- **THEN** the output names the artifact tier and states that `up` runs
  `Package.swift`

#### Scenario: A portable SwiftPM package keeps the closure default
- **WHEN** `init` runs in a directory holding `Package.swift` whose
  sources import nothing Apple-only
- **THEN** the generated manifest declares `provider = "flox"` and the
  output states why `swift` was not selected

#### Scenario: An Apple deliverable with portable sources selects swift
- **WHEN** `init` runs in a directory holding `Package.swift`, sources that
  import nothing Apple-only, and an `Info.plist`
- **THEN** the generated manifest declares `provider = "swift"`

### Requirement: doctor has a swift arm
The system SHALL add a swift arm to `doctor`, selected when the
discovered manifest declares `provider = "swift"`, that probes what
resolution resolves against: the developer directory `xcode-select`
names and whether it exists; the toolchain answering
`swift -print-target-info` (a version string proves nothing about
that), with its flavor — Xcode or Command Line Tools — and version; the
SDK `xcrun` would inject; and, under Xcode, that the license is
accepted **and its record is readable**, since a sandboxed `xcodebuild`
reads `/Library/Preferences/com.apple.dt.Xcode.plist` and treats
unreadable as unaccepted. Each failure SHALL name the command that fixes
it. Off macOS the arm SHALL report the provider as unavailable on the
platform and SHALL NOT report a missing toolchain.

#### Scenario: A healthy Xcode
- **WHEN** `doctor` runs on macOS with Xcode selected, its license
  accepted, and a working toolchain
- **THEN** it reports the developer directory as Xcode, the Swift
  version, the SDK path, and the license as accepted and readable

#### Scenario: A selected directory that no longer exists
- **WHEN** `xcode-select` names a directory that is absent
- **THEN** the arm fails naming `sudo xcode-select -s <dir>`

#### Scenario: License not accepted
- **WHEN** Xcode is selected and `xcodebuild -checkFirstLaunchStatus`
  fails
- **THEN** the arm fails naming `sudo xcodebuild -license accept`

#### Scenario: `doctor` on Linux
- **WHEN** `doctor` runs on Linux and the manifest declares `swift`
- **THEN** it reports the provider as macOS-only and does not suggest
  installing a toolchain

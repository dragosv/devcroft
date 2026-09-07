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

`init` SHALL select `swift` only for a package that imports an Apple-only
module without a conditional-compilation guard, so that it never generates
a manifest the provider would refuse at `up`. Detection SHALL NOT execute
the package's own code.

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

# policy Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: Artifact-tier host grants are attributed and rendered

The system SHALL compile an artifact-tier provider's host library grants
with a `provider:<name>` origin, and `policy --render` SHALL show them
under that origin.

This is the point at which the difference between a self-contained closure
and a host-linked runtime stops being a tier name in documentation and
becomes a visible difference in the compiled policy. A rule that reaches
the backend and cannot be shown by `--render` violates the standing policy
invariant, and an artifact-tier provider is the first case where such
rules are numerous enough for the difference to be worth reading.

#### Scenario: Rendering a Swift sandbox's policy

- **WHEN** `policy --render` runs for a sandbox with
  `env.provider = "swift"`
- **THEN** the developer-directory and dynamic-linker grants SHALL appear
  with origin `provider:swift`
- **AND** the rendered rule set SHALL be byte-identical for identical
  inputs

#### Scenario: Comparing a closure-tier and an artifact-tier sandbox

- **WHEN** `policy --render` is run for a flox sandbox and for a Swift
  sandbox in the same project shape
- **THEN** the Swift output SHALL contain host paths that the flox output
  does not
- **AND** the difference SHALL be attributable to the provider origin
  rather than to the baseline

### Requirement: Grants must name paths the host actually has

The system SHALL NOT emit a filesystem grant for a path that does not
exist at compile time without recording that fact.

On macOS the backend matches paths as spelled, so a grant naming an absent
path is accepted, compiled, and enforces nothing. The system libraries a
Swift artifact links against are served from the dynamic linker's shared
cache and are absent from the filesystem, which makes this the first
provider where the failure is reachable by an obvious, wrong
implementation.

#### Scenario: A grant naming an absent system dylib

- **WHEN** policy compilation is asked to grant a path that does not exist
  on the host
- **THEN** compilation SHALL surface that the path is absent
- **AND** it SHALL NOT silently produce a rule that enforces nothing

### Requirement: Guarantee tier is carried in the compiled policy

The system SHALL record the resolved provider's guarantee tier alongside
the compiled policy, so that `status` and the `up` notice read the tier
from the same source the policy was compiled from.

The tier SHALL NOT be derived independently at each display site. Two
places deriving the same fact is how a compiled policy and its description
come to disagree, which the standing invariant that `--render` renders
from `Meta` exists to prevent.

#### Scenario: Tier is consistent between policy and status

- **WHEN** a Swift sandbox is up
- **THEN** `status` SHALL report the artifact tier
- **AND** the tier reported SHALL come from the same recorded metadata the
  compiled policy was built from

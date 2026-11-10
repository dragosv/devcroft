## Purpose

Give tests a way to drive `up` with a chosen provider without giving anyone
a way to ship one — and without creating a second path through the code that
enforces the sandbox.

## ADDED Requirements

### Requirement: The injected path is the enforcement path

The seam SHALL be the whole of `up` minus provider selection. Every step the
public path performs — deny-overlap validation, the mount-isolation probe,
listener-before-restriction ordering, hook ordering, policy compilation and
its origin attribution — SHALL be performed identically when a test injects
a provider.

Stated as a requirement rather than left to review because the failure is
silent and delayed: a step added later to `up` and not to the injected path
produces a suite that passes while testing a sandbox nobody ships. This
project already designs that class of drift out rather than watching for it —
`resolved_grants` and `to_capability_set` share one resolver precisely so
Landlock's grants and the mount view cannot disagree — and the seam SHALL
follow the same rule: one function, two callers, not two functions that must
be kept in step.

#### Scenario: A test brings up a sandbox through the seam

- **WHEN** a test injects a provider and calls the seam
- **THEN** the sandbox that results is compiled, validated and restricted by
  the same code the published binary runs
- **AND** no enforcement step is skipped because the caller was a test

#### Scenario: A step is added to `up`

- **WHEN** a later change adds a step to the public `up` path
- **THEN** the injected path performs it too, without that change having to
  remember the seam exists

### Requirement: The seam is not reachable from a manifest or the published binary

`ProviderKind` SHALL continue to admit exactly `flox`, `nix` and `devbox`,
and `config::parse` SHALL continue to reject every other `env.provider`
value. The seam SHALL live behind a non-default Cargo feature and SHALL NOT
be compiled into a default build.

devcroft's standing position is that there is no non-reproducible mode and no
passthrough provider. A test seam that could be named in a `devcroft.toml`
would be exactly that exception, whatever it was called.

#### Scenario: A manifest naming a test provider

- **WHEN** a `devcroft.toml` sets `env.provider` to anything outside the three
- **THEN** parsing fails, as it does today
- **AND** the failure distinguishes "not supported" from "out of scope by design"

#### Scenario: A default build

- **WHEN** the crate is built without the test-support feature
- **THEN** the seam is absent from the binary
- **AND** no code path can select a fixture at runtime

### Requirement: Injected rule origins are the row's own

Where a fixture drives `up`, the compiled policy's provider-attributed rules
SHALL carry the origin of the provider that row represents.

`policy --render` and `why` are user-facing surfaces whose vocabulary is
`manifest:` / `provider:<name>` / `baseline`. A fixture that introduced a new
origin token would leak test vocabulary into the output of a shipped binary
and break the tests that assert on the existing ones.

#### Scenario: Rendering a fixture-driven sandbox

- **WHEN** a sandbox is brought up through the seam on a given row
- **THEN** provider-attributed rules render with that provider's own name
- **AND** no origin token exists that a real provider could not produce

### Requirement: The seam covers every provider entry point `up` uses

The seam SHALL account for each way `up` reaches a provider — environment
resolution, the environment fingerprint, the static provider name used for
rule attribution, and the provider-declared service query — not resolution
alone.

Measured, not assumed: `up` reaches the provider through four distinct
entry points, of which only resolution is on the `Provider` trait. A seam
covering resolution alone would leave fingerprinting, attribution and service
declaration on the real-provider path, so a fixture row would exercise a
composition that production never produces.

#### Scenario: A row that declares services

- **WHEN** a fixture row declares services and a test brings the sandbox up
- **THEN** the service declarations reaching the supervisor are the row's own
- **AND** the query does not fall through to a hardcoded provider

#### Scenario: Staleness under an injected row

- **WHEN** a test drifts a row's project and re-reads status
- **THEN** the fingerprint compared is the one that row produces
- **AND** it is recorded in `meta.json` the same way a real `up` records it

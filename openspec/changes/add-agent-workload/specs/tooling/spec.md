# tooling Delta Specification (add-agent-workload)

## Purpose

A second declarative environment composed into a sandbox for tools that
are not project dependencies — a coding agent's runtime being the
motivating case. Lets a tool reach the inside of the boundary without
being declared as something the project itself depends on, while holding
that tool to the same reproducibility bar as the project environment.

## ADDED Requirements

### Requirement: Tooling layer is declarative and locked
The system SHALL require a declared tooling layer to be a declarative,
lockfile-backed environment resolved by a supported provider, held to the
same qualification bar as the project environment. The system SHALL
reject a tooling layer that cannot produce a lock, or that names a
non-reproducible source, at `up` with layer `provider` — never accept it
with a warning. Referencing a binary already present on the host SHALL
NOT be a supported way to populate the tooling layer.

#### Scenario: Unlocked tooling layer rejected
- **WHEN** a `[tools]` layer names an environment with no lockfile
- **THEN** `up` fails at layer `provider` with exit code 3, naming the
  missing lock, and the sandbox does not start

#### Scenario: Host binary is not a tooling layer
- **WHEN** a `[tools]` layer is declared in a way that would pass through
  a binary from the host rather than resolving a closure
- **THEN** it is rejected — the tooling layer is not a reintroduction of
  host passthrough under another name

### Requirement: Tooling resolution happens host-side at up
The system SHALL resolve the tooling layer once, host-side, during the
same trusted provisioning phase as the project environment, before
restrictions are applied. The system SHALL NOT resolve, download, or
activate tooling from inside the boundary, and SHALL NOT require
provider internals to be reachable inside the sandbox.

#### Scenario: Tooling materialized before restriction
- **WHEN** a sandbox with a tooling layer comes up under
  `network.default = "deny"`
- **THEN** the declared tool runs inside the sandbox, because its
  materialization happened host-side at `up`, not at first use

### Requirement: Tooling composes at a fixed, deterministic position
The system SHALL compose the tooling layer's captured environment and
read-only grants with the project environment's at a fixed, documented
position, independent of resolution timing or ordering of I/O. Composing
the same manifests SHALL produce byte-identical results across runs.

#### Scenario: Composition is deterministic
- **WHEN** the same project and tooling layers are resolved repeatedly
- **THEN** the resulting environment and the rendered policy are
  byte-identical each time

#### Scenario: Precedence is stated, not incidental
- **WHEN** the tooling layer and the project environment both provide the
  same binary
- **THEN** which one wins follows the documented precedence, and the
  outcome does not depend on which resolved first

### Requirement: Tooling does not widen the policy
The system SHALL restrict the tooling layer's policy contribution to
read-only store grants carrying a tooling-specific origin. A tooling
layer SHALL NOT add write grants, network grants, or any rule beyond the
read-only closure it resolves. If resolving the tooling layer would
require access the policy does not otherwise permit, `up` SHALL fail
naming the path rather than granting it.

#### Scenario: Rendered policy differs only by read-only grants
- **WHEN** `policy --render` is compared between a manifest with a
  tooling layer and the same manifest without one
- **THEN** the only difference is read-only store grants, each attributed
  to the tooling layer by origin

#### Scenario: Tooling needing write access fails loudly
- **WHEN** resolving a tooling layer would require write access outside
  the project root
- **THEN** `up` fails naming the path, rather than silently widening the
  policy

### Requirement: Tooling is visible in introspection
The system SHALL make a declared tooling layer visible through the same
introspection surfaces that already show environment state, so that a
tool present inside the sandbox is traceable to the layer that provided
it rather than appearing unexplained.

#### Scenario: Tooling grants are attributable
- **WHEN** a tooling layer contributes store grants
- **THEN** `policy --render` shows them with an origin identifying the
  tooling layer, distinct from `provider:` and `manifest:` origins

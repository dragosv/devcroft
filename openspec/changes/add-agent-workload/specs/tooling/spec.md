# tooling Delta Specification (add-agent-workload)

## Purpose

A second, user-declared environment whose `PATH` entries reach a sandbox
for tools that are not project dependencies — a coding agent's runtime
being the motivating case. Lets a tool reach the inside of the boundary
without the project declaring it, while holding that tool to the same
reproducibility bar as the project environment and leaving the project's
environment, `PATH` aside, untouched.

## ADDED Requirements

### Requirement: Tooling layer is declared by the user, never by the project
The system SHALL take a tooling layer only from the user's configuration
file or from `up --tools <path>`, the flag taking precedence over the
file. The system SHALL NOT read a tooling layer from `devcroft.toml` or
from any other file inside the project.

#### Scenario: Layer from the user's configuration
- **WHEN** the user's configuration declares a tooling layer and `up`
  runs in a project whose committed files mention none
- **THEN** the sandbox comes up with the layer's tools on `PATH`

#### Scenario: Flag overrides the configuration
- **WHEN** the user's configuration declares one layer and `up --tools
  <other>` names another
- **THEN** the sandbox uses `<other>`, and the configuration file is not
  modified

#### Scenario: A project cannot declare a layer
- **WHEN** `devcroft.toml` contains a `[tools]` section
- **THEN** validation fails at layer `config` with exit code 2, as for
  any unknown key

### Requirement: Tooling layer is declarative, locked, and closure-tier
The system SHALL require a tooling layer to be a lockfile-backed
environment resolved by a closure-tier provider (flox, nix, devbox,
devenv), regardless of the project's own provider. The system SHALL
reject a layer with no lock, an artifact-tier provider, or a host binary
as its source at `up` with layer `provider`, never with a warning.

#### Scenario: Unlocked tooling layer rejected
- **WHEN** the tooling layer names an environment with no lockfile
- **THEN** `up` fails at layer `provider` with exit code 3, naming the
  missing lock, and the sandbox does not start

#### Scenario: Artifact-tier layer rejected
- **WHEN** the tooling layer names an artifact-tier provider
- **THEN** `up` fails at layer `provider` with exit code 3, naming the
  tier, and the sandbox does not start

#### Scenario: A closure-tier layer serves an artifact-tier project
- **WHEN** a project whose provider is `swift` is brought up with a
  devenv tooling layer
- **THEN** the layer's tools are on `PATH` inside the sandbox

#### Scenario: Host binary is not a tooling layer
- **WHEN** a tooling layer is declared in a way that would pass through
  a binary from the host rather than resolving a closure
- **THEN** it is rejected — the tooling layer is not a reintroduction of
  host passthrough under another name

### Requirement: Tooling resolution happens host-side at up, without hooks
The system SHALL resolve the tooling layer once, host-side, during the
same trusted provisioning phase as the project environment, before
restrictions are applied, through the provider's hook-free route. The
system SHALL NOT run the layer's activation hook, at `up` or inside the
sandbox, SHALL NOT resolve, download, or activate tooling from inside
the boundary, and SHALL NOT require provider internals to be reachable
inside the sandbox.

#### Scenario: Tooling materialized before restriction
- **WHEN** a sandbox with a tooling layer comes up under
  `network.default = "deny"`
- **THEN** the layer's tool runs inside the sandbox, because its
  materialization happened host-side at `up`, not at first use

#### Scenario: The layer's hook never runs
- **WHEN** the tooling layer's environment declares an activation hook
- **THEN** the hook does not run, and nothing it would export reaches a
  session

### Requirement: Tooling contributes PATH entries only, after the project's
The system SHALL add the tooling layer's `PATH` entries to a session's
`PATH`, after the project environment's entries and before devcroft's
resolved shell, in the order the layer's provider reports them. The
system SHALL NOT add any other variable from the tooling layer's
environment to a session. Composing the same project and layer SHALL
produce byte-identical results across runs.

#### Scenario: The project's environment is unchanged
- **WHEN** a sandbox is brought up with and without a tooling layer
- **THEN** every session variable other than `PATH` is identical between
  the two

#### Scenario: The project wins a shared name
- **WHEN** the tooling layer and the project environment both provide
  the same binary
- **THEN** a session resolves the project's

#### Scenario: Composition is deterministic
- **WHEN** the same project and tooling layer are resolved repeatedly
- **THEN** the resulting environment and the rendered policy are
  byte-identical each time

### Requirement: Tooling does not widen the policy
The system SHALL restrict the tooling layer's policy contribution to a
read-only grant of its store root carrying a tooling-specific origin,
rendered even where a provider grant already covers the same path. A
tooling layer SHALL NOT add write grants, network grants, or any rule
beyond that. If resolving the tooling layer would require access the
policy does not otherwise permit, `up` SHALL fail naming the path rather
than granting it.

#### Scenario: Rendered policy differs only by the layer's grant
- **WHEN** `policy --render` is compared between a sandbox with a
  tooling layer and the same project without one
- **THEN** the only difference is a read-only store grant attributed to
  the tooling layer by origin

#### Scenario: Tooling needing write access fails loudly
- **WHEN** resolving a tooling layer would require write access outside
  the project root
- **THEN** `up` fails naming the path, rather than silently widening the
  policy

### Requirement: The layer is recorded, and goes stale like the project
The system SHALL record the tooling layer used at `up`, its path and a
fingerprint of its lock, with the sandbox's metadata, and SHALL render
the layer's grant from that record rather than from the user's
configuration as it currently stands. The system SHALL report a sandbox
as stale when the layer's lock has changed since `up`, or when the
configuration now names a different layer or none, and SHALL apply the
change only on `up --recreate`.

#### Scenario: Configuration changed after up
- **WHEN** the user's configuration is edited to name a different layer
  while a sandbox is up
- **THEN** `policy --render` still shows the layer the sandbox was
  brought up with, and `status` reports the sandbox stale

#### Scenario: Recreate applies the new layer
- **WHEN** `up --recreate` runs after the layer's lock changed
- **THEN** the sandbox is brought up with the layer as it now stands

### Requirement: Tooling is visible in introspection
The system SHALL make a tooling layer visible through the same
introspection surfaces that already show environment state: `up` and
`status` SHALL name the layer in use, `status` SHALL list the layer's
binaries that the project's `PATH` shadows, and `policy --render` SHALL
attribute the layer's grant to it. A tool present inside the sandbox
SHALL be traceable to the layer that provided it rather than appearing
unexplained.

#### Scenario: Tooling grants are attributable
- **WHEN** a tooling layer contributes its store grant
- **THEN** `policy --render` shows it with an origin identifying the
  tooling layer, distinct from `provider:` and `manifest:` origins

#### Scenario: A shadowed binary is shown, not warned
- **WHEN** the project environment and the tooling layer both provide
  `git`
- **THEN** `status` names `git` as shadowed in the layer, and `up`
  prints no warning about it

### Requirement: A fleet resolves the layer once
The system SHALL resolve a fleet's tooling layer once for the fleet and
give every agent the same `PATH` entries, unaffected by where each
agent's clone is mounted.

#### Scenario: Every agent gets the same layer
- **WHEN** a fleet of several agents is brought up with a tooling layer
- **THEN** the layer is resolved once, and each agent's session has the
  same layer entries on `PATH`

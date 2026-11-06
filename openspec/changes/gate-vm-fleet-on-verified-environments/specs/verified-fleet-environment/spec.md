## ADDED Requirements

### Requirement: VM fleet requires a supported reconstructible definition

VM fleet SHALL require hash-pinned recoverable inputs, declared preparation and
services and non-empty verification through a named supported reconstruction
adapter. A local snapshot, image digest, provider signature or self-declared flag
alone SHALL NOT establish eligibility.

#### Scenario: Local image works but its project environment has no usable recipe

- **WHEN** the user requests VM fleet
- **THEN** admission fails with the missing reconstruction capability or definition
- **AND** local use remains available

#### Scenario: Setup downloads an undeclared dependency

- **WHEN** a setup step reaches for the network during the build stage
- **THEN** the host has closed the channel and the step fails attributably
- **AND** strict eligibility is refused until the input is declared by hash

#### Scenario: A base cannot pin its system packages

- **WHEN** a recipe depends on a base without a hash-pinning package mechanism
- **THEN** admission fails naming that adapter gap
- **AND** local use remains available

### Requirement: Verification uses independent clean state

Verification SHALL use a separate VM with declared inputs, fresh private state and
only eligible immutable caches. Reconstruction SHALL fetch declared inputs through
adapter fetchers that verify their hashes and run no project code, then SHALL run
setup with the proxy channel closed by the host. Required service readiness and
verification commands SHALL succeed without skipped required probes. Failure SHALL
preserve the local exploratory instance and record an attributable outcome.

#### Scenario: Local installation was never declared

- **WHEN** a clean build cannot find that dependency
- **THEN** verification fails even though the exploratory instance still builds
- **AND** its local disk remains intact

#### Scenario: Cached materialization is reused

- **WHEN** an eligible immutable toolchain cache is available
- **THEN** it can be reused without copying prior mutable home or service data
- **AND** current setup/readiness/verification still execute and are recorded

### Requirement: Evidence binds the environment to a reference revision

Controller-owned evidence SHALL bind canonical environment inputs, target, the
declared preparation input set, runtime compatibility, the verification commands
and the reference revision they ran on. A change to environment inputs or
verification commands SHALL invalidate it; a source change outside them SHALL NOT.
Secret values SHALL NOT appear in definitions, identity hashes or evidence.

#### Scenario: A lockfile changes after verification

- **WHEN** fleet admission recomputes the environment identity
- **THEN** prior evidence is stale and a new clean run is required

#### Scenario: Source changes without touching environment inputs

- **WHEN** a later commit leaves the environment identity unchanged
- **THEN** environment evidence stands and immutable artifacts are reused
- **AND** per-source checks run for the new source

#### Scenario: A provider's input set is the whole source

- **WHEN** any source file changes
- **THEN** its environment identity changes and evidence does not carry over

#### Scenario: Repository edits a success marker

- **WHEN** it supplies its own verified flag or another project's evidence file
- **THEN** controller-owned matching evidence is still required

### Requirement: Admission pins the exact inputs it validates

Fleet preflight SHALL check every requested source context, runtime capability,
policy and matching successful environment evidence. The launch transaction SHALL
acquire artifact leases and validate their identities before any agent task starts.

#### Scenario: Definition changes between preflight and launch

- **WHEN** launch observes a different identity
- **THEN** that admission is aborted and the changed inputs require verification

#### Scenario: Artifact was collected after a previous verification

- **WHEN** the verified artifact is unavailable
- **THEN** launch fails with missing-artifact or reconstructs and revalidates it
- **AND** it never substitutes an unverified mutable image

### Requirement: Every agent starts from private prepared state

Each admitted agent SHALL have private workspace, home, service data and runtime
credentials. Its declared per-workspace setup, readiness and startup checks SHALL
run for its source before task execution. An environment-class failure SHALL stop
that agent's task. A project-class failure SHALL be recorded and attached to the
task and SHALL NOT stop it. Shared inputs SHALL remain immutable and contain no
personal credentials. Partial launch failure SHALL be explicit and cleaned up per
instance.

#### Scenario: One agent's database initialization fails

- **WHEN** sibling instances are healthy
- **THEN** the failing agent's task does not start and its failure is reported
- **AND** no sibling database is used as a substitute

#### Scenario: The requested source has a failing test

- **WHEN** environment evidence matches and a per-source check fails
- **THEN** the agent's task starts with the failure attached
- **AND** status reports a project-class failure, not an ineligible environment

### Requirement: Agent results are delivered without host Git on guest metadata

Each agent's work SHALL leave as a staged commit series tied to its source. VM
fleet SHALL NOT fetch from a guest clone on the host, and agents SHALL NOT push.
Status SHALL show undelivered results.

#### Scenario: Three agents finish with commits

- **WHEN** the user collects their work
- **THEN** each agent has its own staged series with its base identity
- **AND** no host Git command reads guest metadata

#### Scenario: A fleet with undelivered work is removed

- **WHEN** removal is requested
- **THEN** the agents with undelivered results are named before anything is removed

### Requirement: Verification claims stay within their observed scope

Status SHALL distinguish local availability, reconstruction evidence, staleness,
per-source check results and eligibility, identifying the workload checked. It
SHALL NOT equate test success with universal completeness, bit-identical
application builds or security attestation, nor environment evidence with the
health of a given commit. Interactive runtime divergence SHALL NOT create new
reusable evidence.

#### Scenario: Agent modifies setup after admission

- **WHEN** the modified instance is proposed as the source of another fleet
- **THEN** its previous admission does not qualify the changed definition
- **AND** reconstruction and verification are required for new reusable evidence

## ADDED Requirements

### Requirement: VM fleet requires a supported reconstructible definition

VM fleet SHALL require pinned recoverable inputs, declared preparation/services
and non-empty verification through a supported reconstruction adapter. A local
snapshot, image digest, provider signature or self-declared flag alone SHALL NOT
establish eligibility.

#### Scenario: Local image works but its project environment has no usable recipe

- **WHEN** the user requests VM fleet
- **THEN** admission fails with the missing reconstruction capability or definition
- **AND** local use remains available

#### Scenario: Setup downloads an unpinned latest dependency

- **WHEN** preparation identifies this as a required material input
- **THEN** strict eligibility is refused until that input is fixed and recoverable

### Requirement: Verification uses independent clean state

Verification SHALL use a separate VM with declared inputs, fresh private state and
only eligible immutable caches. Required service readiness and verification commands
SHALL succeed without skipped required probes. Failure SHALL preserve the local
exploratory instance and record an attributable outcome.

#### Scenario: Local installation was never declared

- **WHEN** a clean build cannot find that dependency
- **THEN** verification fails even though the exploratory instance still builds
- **AND** its local disk remains intact

#### Scenario: Cached materialization is reused

- **WHEN** an eligible immutable toolchain cache is available
- **THEN** it can be reused without copying prior mutable home or service data
- **AND** current setup/readiness/verification still execute and are recorded

### Requirement: Evidence binds environment and exact source identities

Controller-owned evidence SHALL bind canonical environment inputs, target,
tooling, effective policy, runtime compatibility and exact source/verification
context. Relevant changes SHALL invalidate success. Secret values SHALL NOT
appear in definitions, identity hashes or evidence.

#### Scenario: Lockfile or source changes after verification

- **WHEN** fleet admission recomputes the identities
- **THEN** prior evidence is stale and checks for the changed context are required
- **AND** unchanged immutable environment artifacts can still be reused

#### Scenario: Repository edits a success marker

- **WHEN** it supplies its own verified flag or another project's evidence file
- **THEN** controller-owned matching evidence is still required

### Requirement: Admission pins the exact inputs it validates

Fleet preflight SHALL check every requested source context, runtime capability,
policy and matching successful evidence. The launch transaction SHALL acquire
artifact leases and validate their identities before any agent task starts.

#### Scenario: Definition changes between preflight and launch

- **WHEN** launch observes a different identity
- **THEN** that admission is aborted and the changed inputs require verification

#### Scenario: Artifact was collected after a previous verification

- **WHEN** the verified artifact is unavailable
- **THEN** launch fails with missing-artifact or reconstructs and revalidates it
- **AND** it never substitutes an unverified mutable image

### Requirement: Every agent starts from private prepared state

Each admitted agent SHALL have private workspace, home, service data and runtime
credentials, and SHALL pass its own declared setup/readiness/startup checks before
task execution. Shared inputs SHALL remain immutable and contain no personal
credentials. Partial launch failure SHALL be explicit and cleaned up per instance.

#### Scenario: One agent's database initialization fails

- **WHEN** sibling instances are healthy
- **THEN** the failing agent's task does not start and its failure is reported
- **AND** no sibling database is used as a substitute

### Requirement: Verification claims stay within their observed scope

Status SHALL distinguish local availability, reconstruction evidence, staleness
and eligibility, identifying the workload checked. It SHALL NOT equate test
success with universal completeness, bit-identical application builds or security
attestation. Interactive runtime divergence SHALL NOT create new reusable evidence.

#### Scenario: Agent modifies setup after admission

- **WHEN** the modified instance is proposed as the source of another fleet
- **THEN** its previous admission does not qualify the changed definition
- **AND** reconstruction and verification are required for new reusable evidence

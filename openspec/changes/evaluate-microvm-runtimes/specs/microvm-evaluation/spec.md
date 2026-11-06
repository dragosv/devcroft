## ADDED Requirements

### Requirement: Candidate runs have immutable comparable inputs

The experiment SHALL run pinned hvi and libkrun 2.0 library adapters with the
same guest artifacts and resource settings within each host/architecture row.
It SHALL record revisions, hashes, runtime options and deliberate differences.

#### Scenario: A candidate cannot boot the common kernel

- **WHEN** one candidate requires a different kernel or feature configuration
- **THEN** the mismatch is recorded as a comparison limitation or blocker
- **AND** results are not presented as measurements of identical guests

#### Scenario: Required hardware is unavailable

- **WHEN** a platform row cannot execute on compatible hardware
- **THEN** its result is not-run with the missing prerequisite named
- **AND** another platform's result does not qualify that row

### Requirement: Both environment shapes exercise real devcroft behavior

Each qualifying adapter SHALL run a declared closure fixture and an adaptable
local fixture, including build/test, service readiness, persistence and the
keeper's exec, PTY, signal, SSH, SFTP and guest-forwarding behaviors.

#### Scenario: Adaptation survives restart but is omitted from the recipe

- **WHEN** a dependency installed locally survives restart and a fresh recipe
  reconstruction lacks it
- **THEN** local persistence passes and reconstruction fails independently
- **AND** the working local instance is not evidence of fleet eligibility

#### Scenario: Agent reaches an API with dummy credentials

- **WHEN** the agent receives an authentication rejection from the intended API
- **THEN** only startup/transport/TLS is recorded as exercised
- **AND** completed task execution is not reported as passed

### Requirement: Qualification requires confinement before guest execution

A qualifying run SHALL establish host confinement before first guest execution,
inventory surviving descriptor authority, and enforce explicit host channels.
Absence of a required control SHALL fail qualification rather than downgrade it.

#### Scenario: vCPUs start during builder construction

- **WHEN** a VMM starts guest execution in its build operation
- **THEN** confinement established only after build is rejected
- **AND** the candidate needs an earlier restriction or demonstrated upstream seam

#### Scenario: Denial test has no functioning positive control

- **WHEN** both forbidden and permitted access fail because transport is broken
- **THEN** the isolation test is inconclusive rather than passed

### Requirement: The experiment probes recovery and host channel abuse

The harness SHALL test keeper/init/helper death, transport loss, forced stop,
restart, host suspend/resume and malformed or oversized guest requests. Cleanup
SHALL verify absence of live orphan helpers and reusable stale endpoints.

#### Scenario: One helper is killed during a write

- **WHEN** the VMM helper receives forced termination
- **THEN** the result is recorded as a crash and its disk recovery is checked
- **AND** sibling instances continue and stale endpoints cannot address them

### Requirement: Density measurements include successful workload and full cost

The report SHALL include 1/4/8-instance raw samples, cache conditions, durability,
all host helper costs, storage allocation and memory before/during/after build.
Failures, exhaustion and not-run samples SHALL remain visible.

#### Scenario: Eight instances start but only six finish the workload

- **WHEN** two instances fail or skip required work
- **THEN** the row is not reported as a successful eight-instance density result

### Requirement: Adoption remains separate from experiment completion

The report SHALL distinguish observed results, source-derived expectations and
unknowns, and SHALL permit selection of neither backend. Production adoption
SHALL require an accepted report and platform-specific release criteria.

#### Scenario: A fast candidate lacks pre-execution confinement

- **WHEN** startup measurements are favorable but the confinement gate fails
- **THEN** the candidate is not recommended for the untrusted-project runtime

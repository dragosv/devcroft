# cli Delta Specification (add-agent-workload)

## ADDED Requirements

### Requirement: --name is accepted wherever a sandbox is resolved
The system SHALL accept `--name` on the commands that resolve a sandbox,
selecting the sandbox identity explicitly instead of deriving it from the
discovered manifest. When `--name` is given, the system SHALL NOT require
the discovered manifest's declared name to match it.

#### Scenario: Name override resolves an existing sandbox
- **WHEN** `exec --name feature-a` runs from a worktree whose manifest
  declares a different name
- **THEN** the command targets `feature-a`, without the
  manifest-mismatch error that an unqualified name argument produces

#### Scenario: Fan-out across worktrees is scriptable
- **WHEN** several worktrees each run `up --name <distinct>`
- **THEN** each produces its own sandbox, with no committed file edited
  in any of them

### Requirement: Credential exposure is disclosed at up
The system SHALL print exactly one line per exposed credential at `up`,
naming the credential and the shape it was delivered in. The disclosure
SHALL appear whether or not the sandbox was already running, and SHALL
NOT be suppressed by the sandbox being idempotently up.

#### Scenario: Disclosure on a repeat up
- **WHEN** `up` runs against an already-running sandbox that exposes a
  credential
- **THEN** the disclosure line is still printed, so exposure is never
  invisible on a subsequent run

### Requirement: doctor reports tooling layer resolvability
The system SHALL report through `doctor` whether a declared tooling layer
can be resolved on this host, distinguishing "no tooling layer declared"
from "declared but not resolvable" and naming the fix in the latter case.

#### Scenario: Tooling layer declared but unresolvable
- **WHEN** `doctor` runs where a declared tooling layer's provider is
  unavailable
- **THEN** it reports the failure and names the fix, rather than leaving
  it to surface as an `up` failure later

#### Scenario: No tooling layer declared
- **WHEN** `doctor` runs for a project with no `[tools]` section
- **THEN** it reports that none is declared, and does not treat the
  absence as a problem

### Requirement: Project-root conflict is actionable
The system SHALL report a project-root conflict as an error naming both
roots, the sandbox name involved, and `--name` as the resolution, at
layer `keeper` or `config` consistent with the error contract, never as a
generic failure.

#### Scenario: Conflict names the fix
- **WHEN** `up` runs from a second worktree under a name already bound to
  another project root
- **THEN** the error names both project roots and instructs the user to
  pass `--name`, with a stable exit code

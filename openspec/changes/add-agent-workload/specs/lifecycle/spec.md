# lifecycle Delta Specification (add-agent-workload)

## ADDED Requirements

### Requirement: Sandbox identity is bound to its project root
The system SHALL record the project root a sandbox was created from, and
SHALL refuse to adopt an existing sandbox's state when `up` runs from a
different project root under the same name. The refusal SHALL name both
roots and the `--name` flag as the resolution. The system SHALL NOT
silently serve one project root's code from a sandbox created for
another.

#### Scenario: Second worktree does not silently share a sandbox
- **WHEN** a git worktree carrying the same committed manifest — and
  therefore the same declared `sandbox.name` — runs `up` while the
  original checkout's sandbox exists
- **THEN** `up` fails naming both project roots and `--name`, instead of
  adopting the existing sandbox

#### Scenario: Same root is still idempotent
- **WHEN** `up` runs twice from the same project root
- **THEN** the second run is idempotent exactly as before — the check
  distinguishes a different root, not a repeated run

#### Scenario: Distinct names coexist
- **WHEN** two worktrees of one repo run `up` with distinct `--name`
  values
- **THEN** both sandboxes exist independently, each recording its own
  project root

### Requirement: Explicit sandbox identity via --name
The system SHALL accept a `--name` override that selects the sandbox
identity independently of the value declared in the manifest, so that
several checkouts of one repository can be brought up simultaneously
without editing a committed file. An overridden name SHALL be used
consistently for state, introspection, and SSH host naming.

#### Scenario: Override does not require editing the manifest
- **WHEN** `up --name feature-a` runs in a worktree whose manifest
  declares a different name
- **THEN** the sandbox is created under `feature-a`, the working tree is
  left unmodified, and the sandbox is reachable under that name

#### Scenario: Overridden name is used everywhere
- **WHEN** a sandbox is created with `--name feature-a`
- **THEN** `status`, `ps`, `logs`, and its SSH host name all use
  `feature-a`

### Requirement: Tooling and credentials resolve before restriction
The system SHALL resolve any declared tooling layer and prepare any
requested credential during the host-side provisioning phase at `up`,
before restrictions are applied, and SHALL fail `up` if either cannot be
satisfied rather than starting a sandbox in which the declared tool or
credential is missing.

#### Scenario: Unsatisfiable credential fails up
- **WHEN** a requested credential file does not exist on the host
- **THEN** `up` fails naming the missing credential, rather than starting
  a sandbox where the agent will fail to authenticate later

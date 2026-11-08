# Policy View Fidelity

## ADDED Requirements

### Requirement: An incomplete policy view SHALL say it is incomplete

`why` and `policy --render` reconstruct their answer from the manifest plus
three contributions that exist only in `Meta`: the provider's
`read_only_grants`, the egress proxy's port, and the service supervisor's
socket. When `Meta` exists but cannot be read, the reconstruction is
incomplete and the resulting verdict can be inverted. The command SHALL NOT
present such an answer as authoritative.

This is distinct from the case the fallthrough was written for: when no
sandbox exists, the manifest-only answer is complete and correct, and MUST
keep its current output unchanged.

#### Scenario: A sandbox exists but its record is unreadable

- **GIVEN** a sandbox whose `meta.json` exists and cannot be read
- **WHEN** the user runs `devcroft why --path <p> --op read`
- **THEN** the command SHALL report that the sandbox's recorded grants could
  not be read, naming the reason
- **AND** it SHALL NOT print a bare `ALLOWED`/`DENIED` verdict as though the
  policy had been fully reconstructed

#### Scenario: No sandbox exists

- **GIVEN** a project with a valid manifest and no state directory
- **WHEN** the user runs `devcroft why --path <p> --op read`
- **THEN** the command SHALL answer from the manifest alone, exactly as it
  does today, with no warning
- **AND** the exit code SHALL be unchanged

#### Scenario: An unreadable record is not confused with a malformed one

- **GIVEN** a sandbox whose `meta.json` is present but not valid JSON
- **WHEN** the user runs `devcroft policy --render`
- **THEN** the failure SHALL be reported distinctly from an unreadable
  record, because the remedies differ

### Requirement: The compiled policy SHALL be readable from inside its own sandbox

`DEVCROFT_DATA_DIR` is baseline-denied and not overridable, so a process
inside a sandbox can never read `Meta`. On Linux it is not even present: the
mount view never contains it, so the read fails with "not found", the same
answer a host with no sandbox gets, and no handling of the read's error can
tell the two apart. Since every in-sandbox invocation
therefore takes the degraded path, honesty alone leaves the in-sandbox query
permanently unable to answer. `up` SHALL write the compiled policy, with
origins, where that sandbox can read it.

The location SHALL be inside the project root — the only path a sandbox both
reads and writes — under the existing artifact directory, and `up` SHALL
leave it ignored by git whether or not `init` ever ran, without editing a
file the user owns. (Written as a self-ignoring `.devcroft/.gitignore`: the
entry `init` writes in the project's own `.gitignore` covers only a project
at its repository's root.)

#### Scenario: A query from inside the sandbox is complete

- **GIVEN** a running sandbox whose provider granted a store path
- **WHEN** a process inside that sandbox runs the `why` query for a path
  under that grant
- **THEN** the verdict SHALL be `ALLOWED` with origin `provider:<name>`
- **AND** it SHALL match, byte for byte in its verdict and origin, what the
  same query returns on the host

#### Scenario: The written copy cannot disagree with what the backend was given

- **GIVEN** an `up` that compiled a policy and derived a `CapabilityPlan`
  from it
- **WHEN** the policy artifact is written
- **THEN** it SHALL be written from that same `CompiledPolicy`, in the same
  operation
- **AND** if it cannot be written, `up` SHALL fail rather than start a
  sandbox whose policy cannot be inspected from within it

#### Scenario: A stale artifact is not trusted

- **GIVEN** a policy artifact left behind by a previous `up` of a sandbox
  that is no longer running
- **WHEN** a policy view falls back to the artifact
- **THEN** it SHALL establish that the artifact belongs to the current
  sandbox instance before using it, and otherwise treat it as absent

#### Scenario: How the artifact is known to be current

- **WHEN** a policy view runs inside a sandbox
- **THEN** it reads that sandbox's artifact only because its `HOME` is that
  sandbox's own home, which devcroft sets and which sits beside the artifact
- **AND** on the host it uses an artifact only where the sandbox's record
  names that artifact's digest, which `up` writes after the artifact

### Requirement: The two views SHALL agree

`why` and `policy --render` compile through one path today, and the fix SHALL
NOT introduce a second. Whatever `--render` shows is what `why` reasons over,
in every context, including from inside the sandbox.

#### Scenario: Render and explain agree in every context

- **GIVEN** any of the three contexts — host with a sandbox, host without
  one, inside the sandbox
- **WHEN** `policy --render` shows a rule granting a path
- **THEN** `why` for that path SHALL report it allowed, with the same origin

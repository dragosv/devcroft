# lifecycle Delta Specification (use-nono-library)

## ADDED Requirements

### Requirement: The keeper restricts itself with no intermediate process
The system SHALL apply the compiled policy from within the keeper
process itself, after the keeper has inherited its listener file
descriptors and before anything project-supplied runs. No process
between `up` and the keeper SHALL be responsible for applying the
restriction, and the keeper SHALL NOT be executed as a child of a
separate sandboxing binary.

#### Scenario: Listeners survive self-restriction
- **WHEN** the keeper applies the compiled policy to itself
- **THEN** the control and SSH sockets created before restriction remain
  reachable from outside the sandbox, and a client connects successfully

#### Scenario: Restriction precedes project code
- **WHEN** a sandbox declaring hooks and services is brought up
- **THEN** the restriction is in effect before any hook or service
  process starts, and no project-supplied process ever runs unrestricted

#### Scenario: No sandboxing binary in the process tree
- **WHEN** the process tree of a running sandbox is inspected
- **THEN** the keeper is a direct child of the process that started it,
  with no intervening sandbox-applying process

### Requirement: The process tier requires no external backend binary
The system SHALL bring up, operate, and tear down a `process` tier
sandbox on a host where no backend binary is installed. The backend
SHALL be a build-time dependency resolved with the rest of the
dependency graph, not a runtime prerequisite discovered on `PATH`.

#### Scenario: Sandbox works with no backend on PATH
- **WHEN** `up`, `exec`, and `down` run on a host with no backend binary
  installed
- **THEN** each succeeds, and the enforcement observed is identical to a
  host where one is installed

#### Scenario: Backend version is pinned, not probed
- **WHEN** the enforcement layer's version is determined
- **THEN** it comes from the resolved dependency graph rather than from
  invoking a binary, so it cannot differ between two hosts running the
  same devcroft build

## MODIFIED Requirements

### Requirement: Listener-before-restriction ordering
The system SHALL create the unix listener sockets before applying any
restriction, and SHALL apply the restriction to the process that
inherited them. Previously the restriction was applied by a separate
binary that then executed the keeper, so the descriptors crossed a
process boundary devcroft did not control; the keeper now applies the
restriction to itself directly. The ordering guarantee is unchanged —
only the process performing the restriction is.

#### Scenario: Ordering holds under the new arrangement
- **WHEN** a sandbox is brought up
- **THEN** the sockets predate the restriction, remain reachable from
  outside it, and the restriction is applied by the keeper itself

#### Scenario: Failure to restrict is fatal
- **WHEN** applying the restriction to the keeper fails
- **THEN** `up` fails at the keeper layer with the stable exit code for
  that layer, and no sandbox is left running unrestricted

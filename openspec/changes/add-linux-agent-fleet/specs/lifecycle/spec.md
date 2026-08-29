# lifecycle Delta Specification (add-linux-agent-fleet)

## MODIFIED Requirements

### Requirement: The keeper restricts itself with no intermediate process

The system SHALL apply the compiled policy from within the keeper process
itself, after the keeper has inherited its listener file descriptors and before
anything project-supplied runs. No process between `up` and the keeper SHALL be
responsible for applying the restriction, and the keeper SHALL NOT be executed
as a child of a separate sandboxing binary.

**In fleet mode**, the supervisor SHALL create each agent's control and SSH
sockets before restriction, and devcroft's own re-executed `devcroft-init`
helper SHALL inherit and apply the prepared policy before it starts that
agent's keeper. No arrangement SHALL permit a project-supplied process to run
before the restriction has succeeded.

**Why this is a modification and not a violation.** The original requirement
rules out an *external sandboxing binary* standing between `up` and the keeper —
it was written when devcroft exec'd `nono wrap`, and its purpose is that no
opaque third-party process owns the restriction. `devcroft-init` is a re-exec of
the same devcroft binary, carrying a policy the supervisor prepared, and it
exists because fleet needs work done between clone and restriction that a
self-restricting keeper cannot do for itself: enter namespaces, complete the
identity handshake, construct the mount view, and become PID 1.

The property the original protects is preserved exactly — nothing
project-supplied runs unrestricted, and no foreign binary owns the boundary. The
mechanism moves by one hop, from "the keeper restricts itself" to "devcroft's
own helper restricts the process that becomes the keeper", for reasons the
single-sandbox case did not have.

#### Scenario: Listeners survive self-restriction

- **WHEN** the keeper applies the compiled policy to itself
- **THEN** the control and SSH sockets created before restriction remain
  reachable from outside the sandbox, and a client connects successfully

#### Scenario: Restriction precedes project code

- **WHEN** a sandbox declaring hooks and services is brought up
- **THEN** the restriction is in effect before any hook or service process
  starts, and no project-supplied process ever runs unrestricted

#### Scenario: No sandboxing binary in the process tree

- **WHEN** the process tree of a running sandbox is inspected
- **THEN** the keeper is a direct child of the process that started it, with no
  intervening sandbox-applying process

#### Scenario: Fleet agent starts

- **WHEN** an agent is started in fleet mode
- **THEN** its control and SSH sockets are created by the supervisor before any
  restriction is applied
- **AND** `devcroft-init` applies the prepared policy before starting the
  keeper, so no project-supplied process runs unrestricted
- **AND** the only process between the supervisor and the keeper is devcroft's
  own re-executed helper, never a separate sandboxing binary

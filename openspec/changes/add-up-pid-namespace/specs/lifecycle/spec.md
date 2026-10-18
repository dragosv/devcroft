# lifecycle

## ADDED Requirements

### Requirement: A sandbox has its own PID namespace where the host allows

On a Linux host that can mount a fresh procfs in a user and PID namespace, the
keeper SHALL run in a PID namespace of its own, under an init process that
reaps orphans and forwards termination signals, with a fresh `/proc` in the
sandbox's view. A host that cannot SHALL degrade to the host's PID namespace
with exactly one warning naming the loss and the remedy.

#### Scenario: Processes outside the sandbox are not visible

- **WHEN** a session lists `/proc`
- **THEN** it sees only processes of its own sandbox

#### Scenario: Stopping the sandbox

- **WHEN** the operator runs `down`
- **THEN** every process in the sandbox's PID namespace ends

#### Scenario: A host that masks /proc

- **WHEN** the host cannot mount a fresh procfs in a user namespace
- **THEN** `up` warns once, naming the loss and the remedy, and proceeds in the host's PID namespace

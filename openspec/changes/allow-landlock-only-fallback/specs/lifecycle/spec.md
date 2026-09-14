## MODIFIED Requirements

### Requirement: Idempotent up
The system SHALL make `devcroft up` idempotent: if a healthy keeper exists,
report it and exit 0; if a dead keeper left state behind (stale pid file,
orphan sockets), clean up and start fresh; `--recreate` SHALL force a full
teardown, re-resolution of the environment, and recompilation of policy.
`--allow-degraded-isolation` SHALL be accepted alongside any of these; it
applies to the `up` it is given to and is recorded with the sandbox it
starts, and a later `up` without it on a healthy sandbox is the
idempotent no-op, not a re-decision.

#### Scenario: Up on a healthy sandbox
- **WHEN** the keeper is alive and responsive
- **THEN** `up` prints the existing status and exits 0 without side effects

#### Scenario: Recovery after host reboot
- **WHEN** state exists but the pid is dead and sockets are orphaned
- **THEN** `up` removes stale state, starts a new keeper, and notes the
  recovery in one line

#### Scenario: Two `up` invocations race for the same sandbox
- **WHEN** two `devcroft up` invocations for the same, not-yet-running
  sandbox run concurrently
- **THEN** exactly one resolves the environment, compiles the policy, and
  starts a keeper

#### Scenario: Up without the flag on a sandbox that was started degraded
- **WHEN** a sandbox was started with `--allow-degraded-isolation` and a
  later `up` for it runs without the flag while the keeper is healthy
- **THEN** `up` reports it as already up, exits 0, and changes nothing
- **AND** `status` continues to report the recorded fallback

## ADDED Requirements

### Requirement: Status reports the isolation actually applied
The system SHALL record, at `up`, whether the sandbox runs with its
filesystem view or under the process backend alone, and `status` SHALL
report a fallback for the sandbox's whole lifetime. The report SHALL come
from what was recorded at that `up`, not from recomputing what the
manifest would compile, since the manifest cannot know the host.

#### Scenario: A degraded sandbox is inspected later
- **WHEN** `status` runs for a sandbox started with `--allow-degraded-isolation`
- **THEN** it prints one line naming the fallback, what it costs, and
  that it was chosen by the flag

#### Scenario: A sandbox started by an older build
- **WHEN** `status` reads state written before this field existed
- **THEN** it reports no fallback, which is what was true of every
  sandbox such a build could start

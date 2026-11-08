# cli Delta Specification (fix-provisioning-hooks)

## ADDED Requirements

### Requirement: up reports an activation hook it could not skip
The system SHALL print exactly one warning at `up` when the resolved
provider's environment defines an activation hook that devcroft could
not capture around, naming the provider, the construct, and that its
code ran on the host outside the sandbox. The warning SHALL be
actionable in the sense the `doctor`/degraded-capability output already
is: it says what happened and what it means, not merely that something
happened.

The warning SHALL be emitted once per `up`, never per session, and
SHALL NOT be emitted for a project whose environment defines no such
hook. It SHALL NOT change `up`'s exit code.

#### Scenario: Warning names the construct
- **WHEN** `up` resolves a provider whose environment defines an
  activation hook that cannot be skipped
- **THEN** one warning is printed naming the provider and the hook, and
  `up` still succeeds

#### Scenario: Silent when there is nothing to report
- **WHEN** `up` resolves an environment with no such hook
- **THEN** no warning about activation hooks is printed

#### Scenario: Not repeated per session
- **WHEN** a sandbox whose environment has such a hook is up, and
  several `exec` sessions run against it
- **THEN** the warning appeared once, at `up`, and no session repeats it

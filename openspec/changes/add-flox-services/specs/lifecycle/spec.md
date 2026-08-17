# lifecycle Delta Specification (add-flox-services)

## ADDED Requirements

### Requirement: Service start ordering at up
The system SHALL start declared services after the keeper is responsive
and after `hooks.post_create`/`hooks.post_start` have run, so that a hook
seeding fixtures runs before services that consume them. The ordering
SHALL be fixed and documented rather than incidental. Starting services
SHALL be skipped entirely when `--skip-hooks` is given, keeping the
"nothing project-supplied runs" guarantee of that flag intact.

#### Scenario: Hooks precede services
- **WHEN** a sandbox declares both `hooks.post_start` and a service
- **THEN** the hook completes before the service is started

#### Scenario: --skip-hooks also skips services
- **WHEN** `up --skip-hooks` runs against a sandbox declaring services
- **THEN** no service is started, and `status` reports services as not
  started rather than as failed

### Requirement: Service teardown precedes keeper exit
The system SHALL stop all supervised services before the keeper exits on
`down` and `rm`, escalating SIGTERM to SIGKILL after a grace period, in
the same manner sessions are terminated. Teardown SHALL leave no service
process alive on the host even when a service ignores SIGTERM.

#### Scenario: Down stops services first
- **WHEN** a sandbox with running services is taken `down`
- **THEN** services receive SIGTERM, then SIGKILL after the grace period,
  and the keeper exits after the last service is gone

#### Scenario: Service ignoring SIGTERM is still reaped
- **WHEN** a service traps and ignores SIGTERM and the sandbox is torn
  down
- **THEN** it is killed after the grace period, and `down` does not
  report success while the process is still alive

### Requirement: Services restart with the sandbox, not across it
The system SHALL start services on every keeper start (matching
`hooks.post_start` semantics, not `post_create`), and SHALL NOT attempt
to preserve service process state across `down`/`up`. Service data that
outlives a restart SHALL do so only through the project's own filesystem,
exactly as it would outside a sandbox.

#### Scenario: Services come back after a restart cycle
- **WHEN** a sandbox with services is taken `down` and brought back `up`
- **THEN** the services are started again, as fresh processes

### Requirement: Status reports service state
The system SHALL include service state in `status`, distinguishing at
minimum running, failed, and not-started, so that a sandbox whose keeper
is healthy but whose services failed is not reported as simply healthy.

#### Scenario: Healthy keeper with a failed service
- **WHEN** the keeper is healthy and one declared service failed to start
- **THEN** `status` shows the keeper healthy and that service failed,
  rather than a single aggregate that hides the failure

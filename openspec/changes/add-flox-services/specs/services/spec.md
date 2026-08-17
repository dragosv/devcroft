# services Delta Specification (add-flox-services)

## Purpose

Long-lived processes (databases, caches, dev servers) declared by the
environment provider's own manifest, started inside a sandbox and
supervised by its keeper for the sandbox's lifetime. Gives each sandbox
its own service instances instead of a shared host one, so parallel
sandboxes cannot corrupt each other's state.

## ADDED Requirements

### Requirement: Services run inside the boundary
The system SHALL start declared services inside the sandbox, after the
compiled policy has been applied, and SHALL subject them to the full
filesystem and network policy. Services are project code and SHALL NOT
receive provisioning privileges under any circumstances. A service
requiring network access SHALL require an allowlist entry exactly as a
session or hook would.

#### Scenario: Service subject to the compiled policy
- **WHEN** a declared service's command tries to reach a host that
  `network.allow` does not include
- **THEN** it is denied by the same policy any session would hit, and the
  denial is attributable to the service in `logs`

#### Scenario: Service cannot escape the filesystem policy
- **WHEN** a declared service's command writes outside the sandbox's
  granted paths
- **THEN** the write is denied, and the policy is not widened because the
  process is a service rather than a session

### Requirement: Keeper supervises services for the sandbox lifetime
The system SHALL track every started service as a supervised process for
as long as the sandbox is up, such that the sandbox can enumerate its
services and their state at any time. Starting services SHALL NOT be
fire-and-forget: a service process that the sandbox cannot later
enumerate and terminate SHALL be treated as a defect, not an accepted
outcome.

#### Scenario: Running services are enumerable
- **WHEN** a sandbox with two declared services is up
- **THEN** both appear with their state through the sandbox's own
  introspection, distinguishable from interactive sessions

#### Scenario: No orphans after teardown
- **WHEN** the sandbox is torn down
- **THEN** no service process started by it remains alive on the host,
  verified by observing process absence rather than by a stop command's
  exit status

### Requirement: Service failure is visible, never silent
The system SHALL report a service that fails to start, or that exits
unexpectedly while the sandbox is up, as a failed service with its log
tail available. A failed service SHALL NOT be presented as healthy, and
SHALL NOT be omitted from service listings.

#### Scenario: Service exits non-zero at startup
- **WHEN** a declared service's command exits non-zero immediately
- **THEN** it is listed as failed with its log tail reachable through
  `logs`, and the sandbox does not report all services healthy

#### Scenario: Service dies while the sandbox is up
- **WHEN** a service that started successfully later exits on its own
- **THEN** its state changes to reflect the exit rather than continuing
  to be reported as running

### Requirement: Services do not block sandbox availability
The system SHALL bring the sandbox to a usable state regardless of
service outcome: a failed service SHALL NOT prevent `exec`, `shell`, or
SSH sessions from working. Service failure SHALL be reported through
service state, not by refusing the sandbox.

#### Scenario: Sandbox usable despite a failed service
- **WHEN** a declared service fails to start
- **THEN** `exec` and `shell` sessions still work, and the failure is
  discoverable through service state

### Requirement: Services and hooks are distinct mechanisms
The system SHALL treat provider-declared services and manifest hooks as
separate mechanisms with distinct contracts: services are supervised,
enumerable, and reaped at teardown; hooks are one-shot and a failing hook
fails `up`. A long-lived process started by a hook SHALL NOT be adopted
as a service, and SHALL remain outside service supervision.

#### Scenario: Hook-started process is not a service
- **WHEN** `hooks.post_start` launches a background process
- **THEN** that process does not appear as a service and is not reaped by
  service teardown — the distinction is stated, not inferred

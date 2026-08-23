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

#### Scenario: The supervisor itself fails
- **WHEN** services are declared but the process supervising them never
  started, or died
- **THEN** the sandbox does not report as having no services: the
  declared services are still listed, and the supervisor's
  unreachability is named

> The failure this rules out: with state read only from the supervisor,
> a supervisor that dies takes every service listing with it, and a
> sandbox with three dead services becomes indistinguishable from one
> that declares none. Reporting requires reconciling live state against
> what was declared, not merely relaying what the supervisor says.

#### Scenario: A service the supervisor never accepted
- **WHEN** a declared service does not appear in the supervisor's own
  listing at all
- **THEN** it is reported as not started, rather than omitted

### Requirement: Service states are distinguished by what actually happened
The system SHALL NOT infer a service's state from a field that does not
carry it. In particular it SHALL NOT report a service that has not run
as having exited, and SHALL NOT attribute to a service an exit code no
process of that service produced.

#### Scenario: A service still waiting to start
- **WHEN** a declared service has been accepted but is waiting on a
  dependency before starting
- **THEN** it is reported as pending, not as exited

#### Scenario: A service skipped because a dependency failed
- **WHEN** a declared service will never run because a service it
  depends on failed
- **THEN** it is reported as skipped, and not as having failed with an
  exit code of its own — the dependency's failure is what is reported as
  a failure

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

### Requirement: Service artifacts belong to one sandbox
The system SHALL keep the generated service configuration, the service
log, and the supervisor's control socket separate per sandbox, so that
two sandboxes sharing a project root do not share, overwrite, or read
each other's. Where those artifacts live is constrained — the state
directory is denied to the sandbox and the supervisor must read its own
config from inside — so separation SHALL come from the path being keyed
on the sandbox, not from choosing a different location.

The system SHALL remove a sandbox's own service artifacts when that
sandbox is removed, and SHALL NOT remove anything belonging to another
sandbox.

The system SHALL fail at layer `config` when the resulting socket path
cannot be bound on this host because of its length, naming the path,
rather than letting the supervisor fail to bind for an unstated reason.

#### Scenario: Two sandboxes, one project root
- **WHEN** two sandboxes with different names are brought up from the
  same project root, each declaring services
- **THEN** each has its own generated config and its own supervisor
  socket, and `status` for each reports only its own services

#### Scenario: Removing one sandbox leaves the other's artifacts
- **WHEN** one of two sandboxes sharing a project root is removed
- **THEN** its own service artifacts are gone and the other's remain

#### Scenario: An unbindable socket path fails loudly
- **WHEN** the project root is deep enough that the supervisor's socket
  path would exceed what the host allows
- **THEN** `up` fails at layer `config` naming the path, rather than
  starting a sandbox whose supervisor cannot bind

### Requirement: Reading the supervisor's socket treats it as untrusted
The system SHALL treat the supervisor's control socket as untrusted
input when read from outside the sandbox: it SHALL verify the path is a
socket owned by the invoking user before connecting, SHALL bound the
size of the response it will accept, and SHALL bound the total time the
exchange may take rather than only the time of any single read.

This matters most at the hardened tier, where the sandbox's own socket
is deliberately made reachable from the host, and the host-side process
reading it is not inside the boundary.

#### Scenario: A non-socket at the expected path
- **WHEN** something that is not a socket exists where the supervisor's
  socket is expected
- **THEN** it is reported as unusable rather than connected to, and
  distinctly from the sandbox simply having no services

#### Scenario: A peer that never stops sending
- **WHEN** the peer streams a response without ending it
- **THEN** the read is abandoned at a bounded size and within a bounded
  time, rather than growing or blocking indefinitely

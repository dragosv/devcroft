# services Delta Specification (add-macos-service-vm)

## ADDED Requirements

### Requirement: Services may run in a shared Linux guest on macOS
The system SHALL support running a sandbox's declared services inside a
shared Linux virtual machine on macOS, while the sandbox's own processes
continue to run natively on the host.

The guest SHALL be shared across sandboxes; the services SHALL NOT be.
Each sandbox SHALL receive its own network namespace, its own supervisor
instance and its own data volume inside the guest.

#### Scenario: Two worktrees run the same declared port
- **WHEN** two sandboxes of one project each declare a service on the same
  port and services run in the guest
- **THEN** both services start, each bound to that port inside its own
  network namespace, and neither `up` fails

#### Scenario: A single shared namespace is refused
- **WHEN** the guest is configured such that two sandboxes' services would
  share one network namespace
- **THEN** the second `up` fails naming the collision, rather than starting
  services that silently share a port table

#### Scenario: Data volumes are per sandbox
- **WHEN** two sandboxes of one project run the same service
- **THEN** each writes to its own data volume, and neither observes the
  other's data

### Requirement: The project's declared service port is never rewritten
The system SHALL start each service on the port the project declared,
inside that sandbox's namespace, and SHALL NOT modify the project's own
service configuration to avoid a collision.

#### Scenario: The manifest stays true in the guest
- **WHEN** a project declares a service on port 5432
- **THEN** that service listens on 5432 inside its sandbox's namespace

### Requirement: The host reaches guest services through a per-sandbox endpoint
The system SHALL expose each sandbox's services to the host through an
endpoint owned by that sandbox — a unix socket where the client supports
one, otherwise a loopback port allocated for that sandbox.

The endpoint SHALL be reported by `status` and injected into the sandbox's
environment. The system SHALL NOT publish two sandboxes' services on the
same host port.

#### Scenario: Endpoints are distinct per sandbox
- **WHEN** two sandboxes both run a service in the guest
- **THEN** each is reachable from the host at a distinct endpoint

#### Scenario: The endpoint is discoverable
- **WHEN** a sandbox's services are running in the guest
- **THEN** `status` names the host-side endpoint for each service

### Requirement: The guest provides Nix and SSH only
The guest SHALL provide a Linux Nix store and SSH access, and SHALL NOT
preinstall any environment tool such as flox, devenv or devbox. A project's
tooling SHALL be materialized into the shared store from the project's own
definition.

#### Scenario: A project's tool is not a guest dependency
- **WHEN** a project changes its environment tool
- **THEN** no change to the guest image is required

### Requirement: Project sources are not mounted into the guest
The system SHALL NOT mount a sandbox's project directory into the shared
guest by default, so that one sandbox's services cannot reach another
sandbox's sources.

#### Scenario: Sources stay on the host
- **WHEN** services run in the guest for a sandbox
- **THEN** the sandbox's project directory is not present in the guest

### Requirement: An unavailable guest is reported, never assumed healthy
The system SHALL report services as unavailable when the guest is not
running, and SHALL NOT report a sandbox healthy on the basis of its
host-side keeper alone.

#### Scenario: The guest is gone
- **WHEN** the guest has stopped and a sandbox declared services
- **THEN** `status` reports the services unavailable

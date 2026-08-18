# port-allocation Delta Specification (add-port-allocation)

## Purpose

Choosing a free loopback port per sandbox so that several sandboxes
created from the same committed manifest can run the same service
without colliding, and making that port discoverable so something can
actually connect to it.

## ADDED Requirements

### Requirement: Allocation is requested by variable, not by number
The system SHALL allow a manifest to request that devcroft choose a
loopback port, identifying the request by the name of an environment
variable that will carry the result. The system SHALL NOT require the
manifest to name a port number for an allocated port, and SHALL NOT
silently reinterpret a fixed port as an allocation request.

#### Scenario: Allocation requested
- **WHEN** a manifest requests allocation for the variable `DB_PORT`
- **THEN** `up` chooses a free loopback port and that variable carries it
  inside the sandbox

#### Scenario: Fixed ports keep their meaning
- **WHEN** a manifest declares a fixed port alongside an allocation
  request
- **THEN** the fixed port is granted exactly as declared, and only the
  allocated one is chosen by devcroft

### Requirement: Allocated ports are granted by the compiled policy
The system SHALL grant an allocated port through the same mechanism a
manifest-declared port uses, and SHALL represent it in the rendered
policy with an origin identifying it as allocated rather than
manifest-declared. An allocated port SHALL NOT be reachable without
appearing in the rendered policy.

#### Scenario: Allocated port is bindable
- **WHEN** a sandbox is up with an allocated port and a deny-default
  network policy
- **THEN** a process inside can bind that port on loopback, and cannot
  bind an unallocated one

#### Scenario: Allocated port is attributable
- **WHEN** `policy --render` runs for a sandbox with an allocated port
- **THEN** the port appears with an origin marking it allocated,
  distinguishable from `manifest:network.ports`

### Requirement: An allocated port is stable for the sandbox's life
The system SHALL record the chosen port and reuse it for the same
sandbox across `down` and `up`, so that a connection string remains
valid while the sandbox exists. The system SHALL choose a new port only
when the sandbox's state is removed, or when the recorded port can no
longer be granted.

#### Scenario: Port survives a restart cycle
- **WHEN** a sandbox with an allocated port is taken `down` and brought
  back `up`
- **THEN** the same port is used, and a client configured against it
  still connects

#### Scenario: Fresh state allocates fresh
- **WHEN** a sandbox is removed with `rm` and created again
- **THEN** a port is chosen anew rather than assumed still free

### Requirement: Allocated ports are discoverable
The system SHALL report every allocated port together with the variable
carrying it, through the introspection surface that already reports
sandbox state. A port that cannot be discovered SHALL be treated as a
defect, not an acceptable outcome: nothing can connect to a value it
cannot read.

#### Scenario: Reported with its variable
- **WHEN** `status` runs for a sandbox with an allocated port
- **THEN** it reports the port and the variable name that carries it

#### Scenario: Two sandboxes report distinct ports
- **WHEN** two sandboxes created from the same committed manifest are
  both up with the same allocation request
- **THEN** each reports a different port, and connecting to one reaches
  that sandbox's process rather than the other's

### Requirement: Allocation reaches the processes that need it
The system SHALL make the allocated value visible to sessions in the
sandbox's environment, and SHALL substitute it into any provider-declared
service configuration devcroft generates, overriding whatever value the
provider's own declaration carried for that variable.

#### Scenario: A service starts on the allocated port
- **WHEN** a provider declares a service whose port comes from a
  variable, and that variable is allocated
- **THEN** the service listens on the allocated port, not on the value
  the provider's manifest declared

#### Scenario: Sessions can read it
- **WHEN** `exec` runs a command in a sandbox with an allocated port
- **THEN** the variable is present in that command's environment

### Requirement: Unsubstitutable ports fail loudly
The system SHALL fail `up` when allocation is requested for a service
that cannot receive it — one whose port is written directly into its
command rather than supplied through the variable being allocated —
naming the service. The system SHALL NOT grant a port that nothing will
listen on, and SHALL NOT rewrite a provider-declared command string to
make it fit.

#### Scenario: Hardcoded port rejected
- **WHEN** allocation is requested but the declared service's command
  contains its port literally and does not reference the allocated
  variable
- **THEN** `up` fails naming the service, rather than allocating a port
  the service will not bind

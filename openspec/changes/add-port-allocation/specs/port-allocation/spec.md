# port-allocation Delta Specification (add-port-allocation)

## Purpose

Choosing a free loopback port per sandbox so that several sandboxes
created from the same committed manifest can run the same service
without colliding, and making that port discoverable so something can
actually connect to it.

## ADDED Requirements

### Requirement: Allocation applies where a collision is possible
The system SHALL apply allocation to sandboxes that share a loopback
with other sandboxes, and SHALL NOT allocate for a sandbox that has its
own network namespace. Sandboxes at the `process` tier, and sandboxes at
the `hardened` tier whose policy grants egress, share the host's
loopback; a `hardened` sandbox with a deny-default network is given its
own network namespace by the generated OCI spec and cannot collide with
another sandbox at all. For those, allocating would replace a
predictable committed port with an unpredictable one and fix nothing.

Where allocation does not apply, a manifest requesting it SHALL still be
valid, and `status` SHALL report the port as the declared value rather
than silently omitting the request.

#### Scenario: Allocated at the process tier
- **WHEN** a sandbox at the `process` tier requests allocation
- **THEN** devcroft chooses a port, and two such sandboxes from the same
  committed manifest receive different ones

#### Scenario: Not allocated when the sandbox has its own loopback
- **WHEN** a sandbox at the `hardened` tier with a deny-default network
  requests allocation
- **THEN** the declared port is used unchanged, and `status` reports it
  as declared rather than allocated

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
longer be bound.

Recording SHALL survive an `up` that does not otherwise change the
sandbox. Whatever mechanism persists per-sandbox facts SHALL be read
before it is rewritten, since a recorded allocation is the first such
fact that cannot be re-derived from the manifest and the environment.

#### Scenario: Port survives a restart cycle
- **WHEN** a sandbox with an allocated port is taken `down` and brought
  back `up`
- **THEN** the same port is used, and a client configured against it
  still connects

#### Scenario: Fresh state allocates fresh
- **WHEN** a sandbox is removed with `rm` and created again
- **THEN** a port is chosen anew rather than assumed still free

#### Scenario: A repeated `up` does not reset the record
- **WHEN** `up` runs again for a sandbox that already has a recorded
  allocation
- **THEN** the recorded port is preserved rather than overwritten by the
  rewrite of per-sandbox state that `up` performs

### Requirement: Losing a recorded port is reported, never silent
The system SHALL report at `up` when a recorded port could not be
reclaimed and a different one was chosen, naming both the old and the
new value. A user or agent holding a connection string SHALL learn that
it is no longer valid from `up` itself, rather than from a failed
connection later.

The system SHALL NOT treat this as a failure of `up`: the recorded value
is a preference, and a sandbox that comes up on a different port is
still usable.

#### Scenario: Reallocation is announced
- **WHEN** a sandbox's recorded port cannot be bound at the next `up`
  because something else on the host now holds it
- **THEN** `up` succeeds, and reports that the allocation changed,
  naming the previous port and the new one

#### Scenario: An unchanged allocation is not announced
- **WHEN** a sandbox's recorded port is reclaimed successfully
- **THEN** `up` reports no allocation change

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
  both up with the same allocation request, on a tier where allocation
  applies
- **THEN** each reports a different port, and a client **running inside
  each sandbox** reaches its own service on its own reported port

> Stated from inside the sandbox deliberately. Where sandboxes share the
> host's loopback, "reaches the right one" holds only because the two
> numbers differ — allocation gives distinct ports, not isolation, and
> nothing stops a host-side process from connecting to either. Where a
> sandbox has its own namespace, a host-side client cannot reach the
> port at all. Neither case supports a promise about host-side
> reachability, so this requirement does not make one.

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

The check SHALL be scoped to the single service the request names, and
SHALL be a test of whether *that* service's command references the
variable. It SHALL NOT be expressed over all declared services: in a
project where only one service uses the allocated variable, every other
service legitimately does not reference it, so any rule quantified over
the whole set either fails every real project or passes the case it
exists to catch.

The system SHALL also fail `up` when a request names a service the
provider did not declare, naming it — a request that can never be
substituted is a manifest error, not a no-op.

#### Scenario: Hardcoded port rejected
- **WHEN** allocation is requested for a service whose command contains
  its port literally and does not reference the allocated variable
- **THEN** `up` fails naming the service, rather than allocating a port
  the service will not bind

#### Scenario: Other services need not reference the variable
- **WHEN** allocation is requested for one service, and other declared
  services do not reference that variable at all
- **THEN** `up` succeeds, because the check concerns only the service
  the request names

#### Scenario: Request for an undeclared service is rejected
- **WHEN** an allocation request names a service the provider did not
  declare
- **THEN** `up` fails naming that service

# service-ports Delta Specification (add-linux-agent-fleet)

## Purpose

Let N agents each run the same declared service — the motivating case is
"every agent gets its own Postgres" — without the agents colliding on a
port, and let a developer reach any individual agent's service from the
host when they need to.

The whole capability rests on one property: **each agent has its own
network namespace, so each has its own port table.** Two agents binding
`5432` are binding two different `5432`s. Nothing needs to negotiate,
allocate, or rewrite anything for the in-namespace case to work.

That makes fleet strictly *easier* here than the single-sandbox case,
which is worth stating because the opposite is the intuitive guess.
`add-port-allocation` exists because today's sandboxes share the host
loopback and therefore need distinct numbers chosen for them, with the
service cooperating by reading its port from a variable. Inside a
namespace none of that applies: the committed port works unchanged, a
port hardcoded in a service's command string is fine, and the service
needs to cooperate not at all. Allocation returns only for the optional
host-side mapping, which devcroft performs by forwarding from *outside*
the agent's namespace and which therefore never requires the service to
know anything.

**The same property now exists outside fleet, for the zero-egress
case.** `CompiledPolicy::wants_network_isolation` gives *any* qualifying
sandbox — not just a fleet agent — its own namespace, so two ordinary
(non-fleet) sandboxes of one project already stop colliding on a
committed port today. What fleet still adds on top is the harder half
this spec is actually about: N agents under one supervisor, and the
optional host-side mapping for reaching a specific one from outside.

## ADDED Requirements

### Requirement: In-namespace ports are authoritative and identical across agents

A service's declared port SHALL be used unchanged inside every agent's
network namespace. The system SHALL NOT allocate, rewrite, or otherwise
vary that port between agents, and SHALL NOT require a service to obtain
its port from an injected variable.

This is the inverse of `add-port-allocation`'s rule, and deliberately so:
that change allocates precisely because a sandbox shares the host
loopback, and its own spec already exempts "a sandbox that has its own
network namespace" from allocation. Fleet is the case that exemption was
written for. Neither change may allocate an in-namespace port; whichever
is implemented second must consume this division rather than restate it.

#### Scenario: Five agents, one declared service

- **WHEN** five agents run from the same repository, whose provider
  manifest declares one service binding a fixed port
- **THEN** all five bind that same port number successfully, each inside
  its own namespace
- **AND** none observes the others' listeners, and no `EADDRINUSE` occurs

#### Scenario: A service hardcodes its port in its command

- **WHEN** a declared service's command contains its port literally
  (`postgres -p 5432`) rather than reading it from a variable
- **THEN** it runs unchanged in every agent
- **AND** no error, warning, or refusal is produced — the constraint that
  makes this a failure for `add-port-allocation` (devcroft does not own
  or rewrite service command strings) does not apply when the port need
  not change

### Requirement: Host-side access is an explicit, per-agent mapping

Reaching an agent's service *from the host* SHALL require an explicit
declaration, and the host-side port SHALL be allocated per agent. The
system SHALL NOT expose an agent's services on the host implicitly.

Forwarding is performed by the supervisor, outside the agent's
namespace. The service is not involved and is not informed: it binds its
declared port inside the namespace and knows nothing about any mapping.

#### Scenario: Two agents, both mapped

- **WHEN** two agents each declare a host mapping for the same service
- **THEN** each receives a distinct host port
- **AND** connecting to one host port reaches that agent's service and not
  the other's

#### Scenario: No mapping declared

- **WHEN** an agent's service declares no host mapping
- **THEN** the service is reachable from inside that agent and from
  nowhere else
- **AND** this is the default: an agent's services are private unless
  something says otherwise

### Requirement: A declared port is devcroft's own configuration, not the provider's

The port a service binds, and any host mapping for it, SHALL be declared
in devcroft's own manifest, keyed by service name. The system SHALL NOT
require a change to the environment provider's manifest schema.

devcroft reads services from the provider's manifest (`add-flox-services`)
and models them as `provider::ServiceDecl` — name, command, per-service
`vars`, daemon flags. That mirrors flox's documented `[services]` schema,
which devcroft consumes and does not own: there is no port field to add,
and adding one upstream is not devcroft's to do. A service's port
therefore lives either inside its command string or inside its `vars`,
neither of which devcroft can reliably parse.

The declaration SHALL share `add-port-allocation`'s configuration
surface — that change already keys its request by service *and* variable
for the same reason — rather than introducing a second, parallel way to
say the same thing.

#### Scenario: Declaring a port for a provider-declared service

- **WHEN** the provider's manifest declares a service and devcroft's
  manifest declares that service's port
- **THEN** the provider manifest is unmodified
- **AND** the two are joined by service name

#### Scenario: A declaration names a service that does not exist

- **WHEN** devcroft's manifest declares a port for a service name the
  provider does not declare
- **THEN** the sandbox fails to start, naming the unmatched service name
- **AND** the failure distinguishes "no such service" from "service
  failed to start", since a typo and a broken service are different
  problems with different fixes

### Requirement: Mappings have a lifetime and are reported

An allocated host mapping SHALL be released when its agent exits, and
SHALL be reported while the agent runs. A mapping SHALL NOT outlive the
agent it belongs to.

#### Scenario: Listing a running fleet

- **WHEN** the operator lists a running fleet
- **THEN** each agent's host mappings are shown alongside the agent, with
  the service each belongs to
- **AND** an agent with no mappings is distinguishable from one whose
  mappings are not yet established

#### Scenario: An agent exits

- **WHEN** an agent stops, cleanly or by crashing
- **THEN** its host mappings are released
- **AND** a later agent may be allocated the same host port without
  conflict

### Requirement: Platforms without namespaces are degraded, not silently different

On a platform that cannot give an agent its own network namespace, the
system SHALL state that in-namespace ports are not isolated, and SHALL
NOT present a shared port as though it were private.

macOS has no network namespace. The declared port is advisory there and
the host mapping is the only real mechanism, which is the same shape a
single-developer macOS user already needs at N=2 — two projects open at
once collide without fleet being involved at all. One schema serves both;
what differs is what it can promise, and that difference SHALL be
surfaced rather than inferred, matching how every other unenforceable
aspect is reported (`policy::degraded`).

#### Scenario: Fleet requested where namespaces are unavailable

- **WHEN** per-agent network namespaces cannot be created on this host
- **THEN** the limitation is named, with the aspect, the reason, and what
  the fallback actually provides
- **AND** it is never reported as though isolation were in effect

### Requirement: Each agent runs its own service stack

Services declared by an agent's provider environment SHALL be started inside
that agent, supervised by that agent's keeper, and contained in that agent's
cgroup leaf. An agent SHALL NOT share service instances with another agent or
with the host.

An agent SHALL NOT be reported ready until its declared services are ready, so
that a task dispatched to a ready agent does not race its own database coming
up.

This is what makes the port isolation above worth having. Two agents each
binding 5432 is only useful if there are two Postgres instances to bind it —
isolation without per-agent services would give each agent its own private way
to reach nothing.

#### Scenario: Two agents, one declared database

- **WHEN** two agents run from a repository whose provider declares a database
  service
- **THEN** each has its own instance, with its own data
- **AND** neither observes the other's, and neither uses the host's

#### Scenario: Agent readiness

- **WHEN** an agent's declared services have not finished starting
- **THEN** the agent is not reported ready
- **AND** a caller waiting for readiness can dispatch work without separately
  polling the services

#### Scenario: A declared service fails to start

- **WHEN** one of an agent's services fails
- **THEN** that agent reports the failure, naming the service
- **AND** other agents are unaffected — a failure is scoped to the agent whose
  service it is, not to the fleet

# agent-networking Delta Specification (add-linux-agent-fleet)

## Purpose

An agent's outbound network reach: why it has none except through a proxy
that decides per destination, and what must stay true for that to hold.

The distinction this capability turns on: the agent's **network namespace**
decides what is reachable at all, and the **proxy** makes the per-hostname
decision. In the shipped topology the namespace has no route beyond its own
loopback, and that absence is the egress boundary, for every protocol and
address (design D9, re-derived and measured). Landlock's `NetPort` is not
that boundary: it scopes by port number alone, never by address, and only
for TCP.

A **network helper** that gives the namespace a general stack (slirp4netns,
pasta; design D5) would remove that boundary, since a workload could open
its own sockets through it. It is not a firewall, and it looks like one. So
a helper and the **proxy-only seccomp filter** (design D9) come together or
not at all: the filter is what a topology with a route needs in place of
the missing boundary, and neither is part of the shipped topology.

## ADDED Requirements

### Requirement: Runtime egress is proxy-only and fails closed

Where an agent is granted runtime egress, the only permitted outbound path
SHALL be that agent's own proxy, reached through a relay to the proxy's
unix socket, which crosses the namespace without a route. Direct sockets to
any other destination SHALL fail at the kernel level, not merely go
unproxied.

Proxy environment variables SHALL NOT be the mechanism. They are advisory
to a cooperating client and say nothing about a workload that ignores them.
The enforcement is that nothing else is reachable.

#### Scenario: Agent opens a direct socket

- **WHEN** a process inside an agent opens a socket to a destination other
  than its proxy relay or a declared listener port inside its own
  namespace, over TCP or UDP, to a loopback or a non-loopback address
- **THEN** the attempt fails at the kernel level
- **AND** it fails whether or not proxy environment variables were set

#### Scenario: Proxy is unavailable

- **WHEN** an agent's proxy is not running
- **THEN** the agent has no egress at all
- **AND** it never falls back to unfiltered access

#### Scenario: Unfiltered egress is requested

- **WHEN** a manifest asks for unfiltered egress (`network.default =
  "allow"`)
- **THEN** fleet refuses to start the agent, naming the key
- **AND** it does not give the agent's namespace a route to carry it

### Requirement: An agent's network namespace has no route out

An agent's network namespace SHALL have no route beyond its own loopback.
This is the invariant the egress requirement above rests on, so a change
that would give an agent a route SHALL be treated as a change to the egress
boundary, not as connectivity plumbing.

A topology that gives agents a route (a general-stack network helper) SHALL
NOT start any agent unless the proxy-only seccomp-notify filter (design D9)
is installed in it before its workload starts, permitting only the agent's
proxy endpoint and its declared listener ports. The filter, and the
listener handoff it needs, are required by such a topology and by nothing
else.

#### Scenario: An agent starts in the shipped topology

- **WHEN** an agent starts
- **THEN** its namespace has its loopback and no route beyond it
- **AND** no network helper runs for it, and no seccomp filter is required
  for its egress to fail closed

#### Scenario: A topology with a route and no filter

- **WHEN** fleet is configured with a network helper that would give an
  agent a route out, and the proxy-only filter is not available
- **THEN** fleet refuses to start agents, naming the missing filter
- **AND** it does not start them relying on the helper's configuration as
  the boundary

### Requirement: Proxy identity comes from the endpoint, not from the client

The proxy SHALL determine which agent a request came from by the endpoint it
arrived on. It SHALL NOT accept an agent identifier supplied by the client.

Anything the client supplies is forgeable by the client, and the client here is
the code being filtered. An identifier in a header would make one agent's
allowlist reachable by another agent that claims its name.

#### Scenario: A request arrives

- **WHEN** a request reaches the proxy from inside an agent
- **THEN** the originating agent is derived from the receiving endpoint
- **AND** nothing in the request content contributes to that determination

#### Scenario: An agent claims another agent's identity

- **WHEN** an agent's request asserts a different agent's identifier
- **THEN** the assertion is ignored
- **AND** the request is evaluated against the allowlist of the agent it
  actually came from

### Requirement: The host is reached only through explicit mappings

An agent SHALL NOT reach any service on the host's loopback. Every inbound
path from the host to an agent SHALL be an explicit, declared mapping
(`service-ports`), and nothing SHALL be forwarded automatically in either
direction.

Where a topology uses a network helper, the helper SHALL be configured to
deny the host's loopback and to forward no ports automatically, and fleet
SHALL verify that by a live probe before the first agent starts.

#### Scenario: Agent reaches for a host-local service

- **WHEN** a process inside an agent connects to an address and port where
  the host is running a service (a database, a dev server)
- **THEN** it does not reach the host's service
- **AND** reaching such a service requires an explicit declaration, not
  merely knowing its port

#### Scenario: Host capability preflight, where a helper is used

- **WHEN** fleet starts under a topology with a network helper, on a host
  whose helper does not accept the required flags or does not enforce them
  as expected
- **THEN** fleet refuses to start, naming what the probe found
- **AND** it does not proceed with a weaker network model

### Requirement: Domain decisions are made by the proxy, never by the agent

Hostname resolution for policy purposes, and the check of a destination against
the allowlist, SHALL happen in the host-side proxy. An agent SHALL NOT be
trusted to resolve, report, or pre-filter its own destinations.

#### Scenario: Agent resolves a name itself

- **WHEN** a process inside an agent resolves a hostname and connects to the
  resulting address
- **THEN** the decision is still made by the proxy, against the name the proxy
  was asked for
- **AND** an address the agent obtained by other means does not bypass the
  allowlist, because the direct socket is refused regardless

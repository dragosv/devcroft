# agent-networking Delta Specification (add-linux-agent-fleet)

## Purpose

An agent's outbound network reach: how it gets any connectivity at all inside
its namespace, and why the only route out is a proxy that decides per
destination.

The distinction this capability turns on: the **network helper** provides a
stack, the **seccomp policy** decides what may be reached, and the **proxy**
makes the per-hostname decision. Collapsing any two of those is the mistake —
in particular, a userspace network helper looks like a boundary and is not one.

## ADDED Requirements

### Requirement: Runtime egress is proxy-only and fails closed

Where an agent is granted runtime egress, the only permitted outbound path
SHALL be that agent's own proxy endpoint. Direct sockets to any other
destination SHALL fail at the kernel level, not merely go unproxied.

Proxy environment variables SHALL NOT be the mechanism. They are advisory to a
cooperating client and say nothing about a workload that ignores them; a
workload that opens its own socket reaches whatever the network helper can
route to unless separately prevented. The enforcement is the proxy-only
seccomp-notify filter (design D9).

#### Scenario: Agent opens a direct socket

- **WHEN** a process inside an agent opens a socket to a destination other than
  its proxy endpoint or a declared listener port
- **THEN** the attempt fails at the kernel level
- **AND** it fails whether or not proxy environment variables were set, and
  whether or not the network helper could have routed it

#### Scenario: Proxy is unavailable

- **WHEN** an agent's proxy is not running
- **THEN** the agent has no egress at all
- **AND** it never falls back to unfiltered access

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

### Requirement: Rootless network plumbing is explicitly bounded

The network helper SHALL be configured to deny access to the host's loopback
and SHALL NOT forward ports automatically. Every inbound forward SHALL be
explicit.

#### Scenario: Agent reaches for a host-local service

- **WHEN** a process inside an agent connects to the host's loopback — a
  database, a dev server, or any other service the operator is running
- **THEN** the connection is refused
- **AND** reaching such a service requires an explicit declaration, not merely
  knowing its port

#### Scenario: Host capability preflight

- **WHEN** fleet starts on a host whose network helper does not accept the
  required flags, or does not enforce them as expected
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

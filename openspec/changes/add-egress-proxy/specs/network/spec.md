# network

> **Authored during integration, not supplied with the change.** The proposal
> names `network` under "Affected specs" but no delta spec accompanied it, and
> a change without one does not validate. Every requirement below is transcribed
> from `proposal.md`'s What Changes / Non-Goals and `design.md`'s E1–E5 rather
> than introduced here — but it is transcription by a second hand, so read it as
> a draft to confirm rather than as the author's own words.
>
> Recorded as **ADDED** rather than the proposal's "MODIFIED `network`": there is
> no `network` capability in this repo today (the capability map is `config`,
> `policy`, `env-provider`, `lifecycle`, `exec`, `ssh`, `cli`, plus `services`
> from add-flox-services). `[network]` is currently a `config` schema section
> compiled by `policy`. Whether egress deserves its own capability or belongs as
> MODIFIED requirements inside those two is a real question this spec's shape
> presumes an answer to.

## ADDED Requirements

### Requirement: Egress leaves only through the proxy

Direct egress from a sandbox SHALL be denied by the kernel-level policy, and the
proxy endpoint SHALL be the only route out. Bypassing the proxy SHALL NOT depend
on the client choosing to cooperate.

Where the proxy is unavailable, the system SHALL fail closed and SHALL NOT fall
back to unfiltered egress.

#### Scenario: Direct connection to an unallowed address

- **WHEN** a process inside the sandbox opens a socket directly to an address
  that is not allowlisted
- **THEN** the connection is refused by the kernel-level policy, not merely left
  unproxied

#### Scenario: Client ignores proxy configuration

- **WHEN** a client that does not honour proxy environment variables attempts to
  connect
- **THEN** it is mediated anyway, because mediation does not depend on the
  client's configuration

#### Scenario: Proxy unavailable

- **WHEN** the proxy cannot be started or has stopped
- **THEN** egress fails closed, and no unfiltered path is substituted

### Requirement: The proxy runs outside the client's sandbox

The proxy SHALL run in the supervisor on the host, outside whatever sandbox the
client runs in, so that the requesting client is identified by the listener the
connection arrived on rather than by anything the client asserts, and so that
credentials the proxy holds are not in the same policy domain as the code being
filtered.

#### Scenario: Attribution without client cooperation

- **WHEN** two sandboxes make requests through the proxy
- **THEN** each request is attributed to its originating sandbox without the
  client supplying any identifier

#### Scenario: Credentials are not reachable from the client

- **WHEN** code inside the sandbox attempts to read the proxy's memory or
  process state
- **THEN** it cannot, because the proxy is outside the sandbox's policy domain

### Requirement: Filtering decides on the requested name

Allowlist entries SHALL be domains, and the decision SHALL be made on the name
the client asked for rather than on port or address alone.

The system SHALL state the limit of this rather than implying more: an
allowlisted name resolves to addresses that may host other services, so the
effective scope may be wider than the name suggests.

#### Scenario: Allowlisted domain

- **WHEN** a connection is requested to a domain on the allowlist
- **THEN** it is permitted

#### Scenario: Domain not on the allowlist

- **WHEN** a connection is requested to a domain that is not on the allowlist
- **THEN** it is refused

#### Scenario: The resolved-address limit is stated

- **WHEN** the enforcement guarantee is documented or reported
- **THEN** it says egress is constrained to allowlisted destinations, and does
  not claim exfiltration is prevented

### Requirement: Network policy is per-context

Provisioning and runtime SHALL carry separate network policies, as they already
carry separate path policies. Neither SHALL be derived from the other.

#### Scenario: Provisioning allowlist differs from runtime

- **WHEN** a manifest declares one allowlist for provisioning and another for
  runtime
- **THEN** each context is enforced against its own, and neither inherits the
  other's entries

#### Scenario: Both contexts are inspectable

- **WHEN** the compiled policy is rendered
- **THEN** both contexts' allowlists appear, each with its origin

### Requirement: Refusals are legible

A refused connection SHALL report the destination and the rule that decided it,
to the operator and — where the protocol allows — to the client.

#### Scenario: A developer sees which host was refused

- **WHEN** a package manager's request is refused
- **THEN** the destination and the deciding rule are reported, rather than the
  failure surfacing only as a generic network error or a timeout

#### Scenario: Refusals are attributable in a fleet

- **WHEN** several sandboxes are running and one is refused
- **THEN** the record names the originating sandbox alongside the destination
  and the rule

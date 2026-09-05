# Brokered Credentials

## ADDED Requirements

### Requirement: A brokered credential SHALL NOT enter the sandbox

The credential exists in the proxy process, which runs unsandboxed on the host.
A sandboxed process reaches the upstream by calling a local route; the proxy
attaches the real credential when forwarding. The sandbox SHALL have no path to
the secret — not through its environment, not through its filesystem, not
through its process table.

This is the position `docs/decisions.md` already states — secrets never via
mounted files or plain environment variables — made achievable rather than
retracted.

#### Scenario: The secret is absent from the sandbox

- **GIVEN** a sandbox configured with a brokered route for an upstream API
- **WHEN** a process inside it enumerates its environment, reads every path it
  is granted, and inspects its own command line
- **THEN** the credential SHALL NOT appear in any of them

#### Scenario: The request still reaches the upstream authenticated

- **GIVEN** the same sandbox
- **WHEN** a process inside it calls the local route
- **THEN** the upstream SHALL receive the request with the credential attached
- **AND** the response SHALL reach the caller unchanged

#### Scenario: Brokering does not intercept TLS

- **GIVEN** a brokered route to an `https` upstream
- **WHEN** the proxy forwards a request
- **THEN** it SHALL establish its own connection to the upstream rather than
  presenting a generated certificate to the sandboxed client
- **AND** no certificate authority SHALL be installed into the sandbox

### Requirement: A brokered route SHALL be declared, never inferred

devcroft compiles policy deterministically and every rule carries an origin. A
route that lends the sandbox an identity is a grant like any other: it SHALL
appear in the manifest, SHALL be visible through `policy --render`, and SHALL
carry an origin.

#### Scenario: The route is inspectable

- **GIVEN** a manifest declaring a brokered route
- **WHEN** the user runs `policy --render`
- **THEN** the route SHALL be shown, with its upstream and its origin
- **AND** the credential's *value* SHALL NOT be shown

#### Scenario: An undeclared upstream is not reachable through the proxy

- **GIVEN** a sandbox whose manifest declares one brokered route
- **WHEN** a process inside it requests a different upstream by path
- **THEN** the proxy SHALL refuse, and the refusal SHALL be distinguishable
  from an upstream error

### Requirement: A missing credential SHALL fail at `up`, not at first use

A brokered route whose credential cannot be resolved on the host SHALL fail
`up` at layer `provider`, before any sandbox is started. A sandbox that comes up
believing it has an identity it cannot obtain has deferred a configuration error
into an agent's runtime, where it surfaces as a confusing upstream failure.

#### Scenario: The credential cannot be loaded

- **GIVEN** a manifest declaring a brokered route whose credential is absent
  from the host
- **WHEN** the user runs `up`
- **THEN** it SHALL fail, naming the route and the missing credential
- **AND** no sandbox SHALL be left running

### Requirement: A client that cannot use the route SHALL be told, not silently degraded

Brokering requires the client to speak plaintext HTTP to a local endpoint. A
client that honours only a proxy variable issues CONNECT to the real upstream
and speaks end-to-end TLS, which cannot be injected into without interception —
an explicit non-goal. devcroft SHALL NOT respond to that by letting the client
carry its own credential instead: the route was declared precisely to prevent
that.

The route SHALL be declared by upstream provider, not by agent, and the
environment variable that points a client at it SHALL be overridable, since no
single naming convention covers every SDK.

#### Scenario: A client that ignores the route is refused, legibly

- **GIVEN** a sandbox with a brokered route, and a client that dials the real
  upstream directly
- **WHEN** the request is attempted
- **THEN** it SHALL fail
- **AND** the failure SHALL say the upstream is brokered and the route was not
  used, rather than reporting only that egress was denied

#### Scenario: The route serves any client following the provider's convention

- **GIVEN** a brokered route declared for an upstream provider
- **WHEN** any client using that provider's standard SDK runs in the sandbox
- **THEN** it SHALL be brokered without devcroft naming that client
- **AND** where the SDK does not follow the convention, the manifest SHALL be
  able to name the variable explicitly

### Requirement: Refused capabilities SHALL remain unreachable

`nono-proxy` compiles in TLS interception, SPIFFE and AWS routing. devcroft
refuses all three. They SHALL NOT be reachable through any devcroft manifest
key, environment variable, or default.

#### Scenario: No configuration path enables them

- **GIVEN** any devcroft manifest that parses
- **WHEN** the proxy configuration is constructed from it
- **THEN** TLS interception, SPIFFE and AWS routing SHALL be disabled
- **AND** a test SHALL assert this, so enabling one becomes a deliberate edit
  rather than a default that drifted

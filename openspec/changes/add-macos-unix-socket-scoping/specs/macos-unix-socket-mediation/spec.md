## Purpose

Give macOS the same guarantee `add-mount-isolation`'s `filesystem-view` gives Linux —
a sandbox cannot reach a pathname unix socket it was not granted — through the
mechanism Seatbelt actually has (network-outbound classification) rather than a
namespace macOS does not.

## ADDED Requirements

### Requirement: A deny-default sandbox cannot reach an ungranted pathname unix socket

On macOS, a sandbox compiled with `network.default = "deny"` SHALL NOT be able to
`connect()` to a pathname unix socket it was not explicitly granted, regardless of
that socket's filesystem permissions.

This is the same class of gap `filesystem-view`'s own first requirement closes on
Linux, closed here through Seatbelt's classification of unix-socket `connect()` as
network-outbound activity rather than filesystem access — the platform's own network
deny rule is what mediates it, not a filesystem-level grant.

#### Scenario: A package-manager daemon socket

- **WHEN** a sandbox with `network.default = "deny"` runs on macOS, and the manifest
  grants no path to a world-accessible unix socket present on the host
- **THEN** the sandbox cannot connect to it
- **AND** the refusal comes from the network deny rule, not a filesystem rule

#### Scenario: A sandbox with `network.default = "allow"`

- **WHEN** a sandbox does not set `network.default = "deny"`
- **THEN** this requirement does not apply to it, matching the existing scope of every
  other network-axis guarantee devcroft makes — an allow-default sandbox is not
  network-isolated in any dimension

### Requirement: The sandbox's own egress path stays reachable

Where a sandbox has an egress proxy, its compiled policy SHALL admit that sandbox's
own path to that proxy, and no other sandbox's, even though the sandbox's own
`network.default = "deny"` denies outbound connections generally.

Stated as an **outcome rather than a mechanism**, for the identical reason
`filesystem-view`'s M3 requirement exists at all: the obvious hardening — denying
everything outright — would silently remove the one path the sandbox itself depends on
for filtered egress, and the symptom would be a sandbox that starts, reports healthy,
and has no network. Which mechanism satisfies it is a platform question and is
deliberately not fixed here. An earlier draft of this requirement mandated a scoped
*unix-socket* grant; that was measured to be the wrong mechanism for macOS, where the
proxy is reached over TCP loopback and no unix socket is dialled by path at all (see
design.md S2). A requirement that names a mechanism can be satisfied and still be
wrong; this one names the property that has to hold.

#### Scenario: An isolated sandbox with an allowlist

- **WHEN** a macOS sandbox has `network.default = "deny"` and a declared
  `network.allow`
- **THEN** it reaches its allowlisted hosts through the proxy
- **AND** what makes that possible is an explicit grant for its own proxy endpoint,
  not an exception to the deny-default requirement above

#### Scenario: Another sandbox's proxy

- **WHEN** a sandbox's policy is compiled
- **THEN** it admits that sandbox's own proxy endpoint and no other sandbox's

#### Scenario: A grant is scoped to what it names

- **WHEN** the policy admits one specific pathname unix socket
- **THEN** a different socket — including one in the same directory — remains denied

### Requirement: A sandbox can create the unix sockets it is expected to run

On macOS, `bind(2)` on a pathname unix socket is `network-bind` — the same
network axis as `connect(2)` — so a `network.default = "deny"` sandbox cannot
create a unix socket at all unless its compiled policy says so. Where devcroft
expects a sandbox to run a component that creates its own control socket, the
compiled policy SHALL admit that socket for both bind and connect.

Stated separately from the egress requirement above because it is the same trap
arriving from the other direction, and because a *filesystem* grant does not
imply it: the service supervisor's socket lives inside the project root, which
the manifest already grants read-write, and it was still refused. Filesystem and
unix-socket authorization are orthogonal layers.

#### Scenario: A sandbox with declared services

- **WHEN** a macOS sandbox has `network.default = "deny"` and its provider
  declares services
- **THEN** the supervisor can bind its own control socket and the services start
- **AND** the grant is present only when services will actually be started

#### Scenario: The grant is inspectable

- **WHEN** such a sandbox's policy is rendered
- **THEN** the socket appears in `policy --render`, like every other compiled
  rule — nothing reaches the backend that cannot be shown

### Requirement: The mechanism is verified before it is claimed

This capability SHALL NOT be reported as enforced — in `docs/known-gaps.md`,
`docs/threat-model.md`, `devcroft doctor`, or any other user-facing surface — until it
has been measured live on macOS hardware, not inferred from the sandbox library's
source alone.

This project's own standing rule (`docs/decisions.md`, `add-backend-capabilities`'s own
`unverified` status) is that a claim believed true from reading a mechanism is not the
same claim as one measured to be true, and this project has shipped that substitution
before and been wrong. Nothing about this capability is exempt from that rule merely
because the reasoning behind it is unusually well-sourced.

#### Scenario: Before the macOS spike runs

- **WHEN** no live measurement on macOS hardware has yet confirmed this capability
- **THEN** `docs/known-gaps.md` continues to state the AF_UNIX gap as open on macOS
- **AND** no document describes it as closed on the strength of the source-reading
  alone

#### Scenario: After the macOS spike confirms it

- **WHEN** a live measurement confirms `network.default = "deny"` denies an ungranted
  pathname unix socket and the scoped proxy-socket grant admits the sandbox's own
- **THEN** `docs/known-gaps.md` and `devcroft doctor`'s capability matrix are corrected
  in the same change that recorded the measurement

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

Where a sandbox has an egress proxy, its compiled policy SHALL include an explicit,
scoped grant admitting `connect()` to that proxy's own unix socket, even though the
sandbox's own `network.default = "deny"` would otherwise deny it.

Stated as a requirement rather than left to the implementation for the identical
reason `filesystem-view`'s M3 requirement is: the obvious hardening — denying every
unix socket outright — would silently remove the one the sandbox itself depends on for
filtered egress. The symptom would be a sandbox that starts, reports healthy, and has
no network.

#### Scenario: An isolated sandbox with an allowlist

- **WHEN** a macOS sandbox has `network.default = "deny"` and a declared
  `network.allow`
- **THEN** it reaches its allowlisted hosts through the proxy
- **AND** the scoped grant admitting the proxy socket is what makes that possible, not
  an exception to the deny-default requirement above

#### Scenario: Another sandbox's proxy socket

- **WHEN** a sandbox's policy is compiled
- **THEN** the scoped grant admits its own proxy socket and no other sandbox's

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

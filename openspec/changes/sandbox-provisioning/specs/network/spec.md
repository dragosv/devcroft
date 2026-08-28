# network

## ADDED Requirements

### Requirement: Network policy is declared per context

Provisioning and runtime SHALL carry separate network policies, and neither
SHALL be derived from the other.

This requirement was written for `add-egress-proxy` and moved here when that
change shipped. The reason is this change's own dependency argument
(`design.md`, open question 2): a per-context policy needs two contexts, and
until provisioning runs inside a boundary of its own there is exactly one.
`add-egress-proxy` built the mechanism a context's allowlist is enforced with —
a resident proxy plus a deny-by-default kernel gate — and shipped it for the
runtime context alone. Declaring a provisioning allowlist that nothing enforces
would have been dead configuration; this change is where the second context
starts existing, so it is where the requirement becomes testable.

#### Scenario: Provisioning permits registries, runtime does not

- **WHEN** provisioning allows package registries and runtime allows nothing
- **THEN** activation reaches those registries
- **AND** the agent, once running, does not

#### Scenario: Rendering

- **WHEN** the operator renders policy
- **THEN** each context's allowlist is shown with its origin
- **AND** a context whose network reach exceeds another's is visibly so

#### Scenario: Provisioning inherits nothing by default

- **WHEN** the manifest declares a runtime `network.allow` and no provisioning
  allowlist
- **THEN** provisioning does not silently receive the runtime allowlist
- **AND** the effective provisioning policy is visible in `policy --render`
  rather than implied

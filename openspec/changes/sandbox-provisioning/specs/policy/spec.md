# policy

## ADDED Requirements

### Requirement: Package-manager authority renders separately from filesystem grants

`policy --render` SHALL show package-manager materialization authority — a
`nix-daemon` socket or equivalent — as its own entry, distinct from the
filesystem grants. It SHALL NOT be represented as a broad write grant on the
store, and SHALL NOT be inferable only from the absence of one.

This exists because the two are trivially confusable and the confusion is
one-directional: "materialization needs to write to the store" reads as a
filesystem need, and satisfying it as one would grant write access to a store
every environment on the machine reads from. Rendering it as its own capability
is what makes a profile that can materialize visibly different from a profile
that merely reads a closure — which is the same reason `network.proxy` renders
separately from `network.ports` rather than being implied by them.

#### Scenario: Rendering a provisioning profile that can materialize

- **WHEN** the operator renders a provisioning profile holding daemon authority
- **THEN** that authority appears as its own entry with its origin
- **AND** the store itself is shown as a read-only grant, not a writable one

#### Scenario: Rendering a runtime profile

- **WHEN** the operator renders a runtime profile
- **THEN** no package-manager authority is shown, because runtime never holds it
- **AND** the resolved store paths still appear as read-only grants

### Requirement: A profile exposing daemon authority to project code is rejected

A provisioning profile that would place a host-global package-manager socket
within reach of project-supplied activation code SHALL be rejected at
validation, unless an operation-scoped mediator has been qualified for that
provider.

Rejection is at compile time rather than a runtime denial for the same reason
`filesystem.deny` overlapping an allow is: the resulting sandbox would be
silently weaker than the manifest reads, and a policy that cannot be enforced as
written must fail rather than compile.

#### Scenario: Profile grants the daemon socket where a hook will run

- **WHEN** a provisioning profile would expose the package-manager socket to a
  context in which project activation code executes
- **THEN** compilation fails, naming the socket and the context
- **AND** the failure distinguishes "this provider cannot be confined" from
  "this profile is misconfigured", since only one of the two is the user's to
  fix

#### Scenario: A qualified mediator is in place

- **WHEN** devcroft mediates the package-manager through an interface that
  scopes it to specific operations, and that mediator has been qualified for
  the provider in question
- **THEN** the profile is accepted
- **AND** what the mediator permits is itself rendered, so the narrower
  authority is inspectable rather than taken on trust

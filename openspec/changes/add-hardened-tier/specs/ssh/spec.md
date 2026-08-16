# ssh Specification

## Purpose

Make the SSH server's *location* tier-dependent without changing what it
exposes or what actually guards it. `add-mvp-core`'s "Embedded server
inside the boundary" requirement was written when the `process` tier was
the only tier, and it conflated two things: where the listener process
runs, and what enforces access to it. Only the first is tier-dependent —
the second (the socket's filesystem permissions, never TCP) is invariant
across tiers and stays exactly as specified.

This delta is backend-generic on purpose: it says a hardened backend with
a native exec-into primitive runs the server host-side, without naming
gVisor. `add-gvisor-backend` supplies the concrete primitive (`runsc
exec`) against this contract, the same way `add-nix-provider` supplied a
concrete provider against `env-provider`.

## MODIFIED Requirements

### Requirement: Embedded server inside the boundary
The system SHALL run an SSH server listening ONLY on a unix socket in the
sandbox state dir, mode 0600, within a 0700 state dir. The server MUST NOT
bind any TCP port.

Where that server process runs is determined by the tier:

- At the `process` tier, and at any hardened tier whose backend has no
  native exec-into primitive, the server SHALL run inside the keeper,
  inside the sandbox boundary, as today.
- At a hardened tier whose backend provides a native exec-into primitive,
  the server SHALL run host-side and dispatch every session through that
  primitive, so sessions still execute inside the boundary while no
  keeper runs inside the sandbox.

In both cases the state dir's filesystem permissions remain the real
access boundary, and observable behavior through `devcroft proxy` SHALL
be identical.

#### Scenario: No TCP exposure
- **WHEN** the sandbox is up, at either tier
- **THEN** no listening TCP socket belongs to the SSH server

#### Scenario: Process tier keeps the server inside the keeper
- **WHEN** the tier is `process`
- **THEN** the SSH server runs inside the keeper, inside the restricted
  process tree, exactly as it does today

#### Scenario: Hardened tier with a native exec primitive
- **WHEN** the tier is `hardened` and the resolved backend provides a
  native exec-into primitive
- **THEN** the SSH server runs host-side on the same 0600-socket-in-0700-
  dir, no keeper runs inside the sandbox, and each session is dispatched
  into the sandbox through that primitive

#### Scenario: Client cannot tell the difference
- **WHEN** the same editor or SSH client connects through
  `devcroft proxy` to a `process`-tier and a `hardened`-tier sandbox
- **THEN** authentication, session behavior, and channel features are
  identical; only `status`'s tier line differs

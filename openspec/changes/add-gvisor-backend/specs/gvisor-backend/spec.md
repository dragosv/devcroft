# gvisor-backend Specification

## Purpose

The runsc adapter behind `isolation = "hardened"`: platform selection,
rootfs synthesis from the provider closure, store sharing, policy
projection onto the OCI mount model, netstack semantics, and session
execution. This capability is deliberately separate from
`add-hardened-tier`'s backend-generic deltas (tier selection, keeper
conditionality, policy targeting) so the two changes compose without
colliding: that change defines what any hardened backend must provide;
this one defines how gVisor provides it.

## ADDED Requirements

### Requirement: Backend resolution and platform selection
When `isolation = "hardened"` resolves to gVisor, the system SHALL select
the runsc platform automatically: systrap by default, KVM when `/dev/kvm`
is accessible and usable. The system SHALL NOT use the deprecated ptrace
platform. `status` SHALL name both the backend and the selected platform.

#### Scenario: Default platform
- **WHEN** a hardened sandbox comes up on a Linux host without usable KVM
- **THEN** runsc runs with the systrap platform and `status` shows
  `isolation: hardened (gvisor/systrap)`

#### Scenario: KVM available
- **WHEN** `/dev/kvm` is present and accessible to the invoking user
- **THEN** runsc runs with the KVM platform and `status` names it

### Requirement: Rootfs synthesized from the provider closure
The system SHALL construct the OCI bundle at `up` from the provider's
resolved environment, not from an image: a minimal rootfs skeleton plus
bind mounts — the store root read-only, the project root read-write, and
nothing else beyond what the compiled policy grants. The bundle SHALL be
reproducible from the same manifest and lockfile, and `up --recreate`
SHALL rebuild it.

#### Scenario: No image, no registry
- **WHEN** a hardened sandbox comes up for a project with a resolved nix
  or flox environment
- **THEN** no container image is pulled or built; the bundle references
  the host's store via a read-only mount and materialization has already
  happened host-side at `up`, before the sandbox exists

#### Scenario: Unmounted paths do not exist
- **WHEN** a path is neither granted by the manifest nor part of the
  provider's store grants nor the baseline skeleton
- **THEN** it is absent inside the sandbox — not present-but-denied —
  because the OCI mount model is deny-by-default

### Requirement: Policy projection preserves origins and rendering
The system SHALL project the same `CompiledPolicy` the process tier
compiles into (a) the bundle's mount list and (b) a Landlock profile
applied to the Sentry process as defense in depth. Rule origins
(`manifest:<key>` / `provider:<name>` / `baseline`) SHALL be unaffected,
and `policy --render` and `why` SHALL produce identical output for the
same manifest regardless of tier.

#### Scenario: Render is tier-independent
- **WHEN** the same manifest is rendered under `isolation = "process"`
  and `isolation = "hardened"`
- **THEN** `policy --render` output is identical, including origins

#### Scenario: Sentry is itself confined
- **WHEN** a hardened sandbox is running
- **THEN** the Sentry process operates under a Landlock profile compiled
  from the same policy, so a compromised Sentry's filesystem reach stays
  bounded by the same grants the sandbox has

### Requirement: Netstack network semantics
Each hardened sandbox SHALL have its own network stack. Under
`network.default = "deny"`, the system SHALL still permit binding and
listening on loopback inside the sandbox — the listener is local to the
sandbox's own netstack and reaches nothing on the host — while denying
egress. Reaching an inside listener from the host SHALL require explicit
forwarding.

#### Scenario: Dev server binds under deny-all
- **WHEN** `network.default = "deny"` and a process inside the sandbox
  binds `127.0.0.1:8080` and listens
- **THEN** the bind succeeds — unlike the process tier today, where the
  same call fails with `EPERM` (the tracked listen-socket gap)

#### Scenario: Same port, two sandboxes
- **WHEN** two hardened sandboxes each bind `:3000`
- **THEN** both succeed, because each bind lands in its own netstack;
  there is no shared host port to conflict over

#### Scenario: Egress still denied
- **WHEN** `network.default = "deny"` and a process inside the sandbox
  connects outward to a host or external address
- **THEN** the connection is denied, and `why --host` explains it with
  the same origin vocabulary as the process tier

### Requirement: Sessions via native exec
The system SHALL run sessions (exec, shell, SSH-spawned) through the
backend's native exec-into primitive (`runsc exec`), with pty
allocation, signal forwarding, exit-code propagation, and the resolved
environment injection behaving identically to the process tier. The
listener-before-restriction fd-passing sequence is not used at this
tier.

#### Scenario: Semantics match the process tier
- **WHEN** the same `devcroft exec`/`shell`/`ssh` workflows run against
  a hardened sandbox
- **THEN** observable behavior (cwd mapping, env, signals, exit codes,
  pty resize) matches the process tier exactly; only `status`'s tier
  line and performance differ

### Requirement: Provider grants map onto mounts or fail loudly
The system SHALL verify at `up` that every read-only grant the provider
resolution emits is representable as a bundle mount, and SHALL fail at
layer `provider` naming the path if one is not — never silently widening
the mount set nor silently dropping a grant (the existing "provider
resolution must not widen the policy" and "degraded capabilities are
surfaced" invariants, projected onto the mount model).

#### Scenario: Grant outside the mountable set
- **WHEN** a provider's resolution emits a grant the bundle cannot
  express
- **THEN** `up` fails naming the path and the reason, rather than
  starting a sandbox whose rendered policy disagrees with what is
  enforced

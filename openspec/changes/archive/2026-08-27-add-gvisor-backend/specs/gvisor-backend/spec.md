# gvisor-backend Specification

## Purpose

The runsc adapter behind `isolation = "hardened"`: platform selection,
rootfs synthesis from the provider closure, store sharing, policy
projection onto the OCI mount model, network policy enforcement, and
session execution. This capability is deliberately separate from
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

### Requirement: Network policy enforcement
The system SHALL NOT use gVisor's per-sandbox netstack (`--network=
sandbox`): upstream `runsc` rejects that mode combined with `--rootless`,
and devcroft runs unprivileged by construction, so the two are mutually
exclusive here. Instead, when `network.default = "deny"` and no
allowlist grants egress, the system SHALL run the sandbox with
`--network=none` (no connectivity of any kind). When the manifest's
`[network]` section grants egress, the system SHALL run with `--network=
host` (gVisor's hostinet passthrough, a mode rootless mode accepts) and
enforce the same `[network]` policy via Landlock's TCP bind/connect
restrictions applied to the Sentry process, with the same rule origins
and `why --host` vocabulary the process tier already uses. This tier
does NOT close the listen-socket/port-conflict gap tracked for the
process tier: `--network=host` shares the host's network namespace, so
a bind inside the sandbox is a real bind on the host, exactly as it is
at the process tier today. That gap staying open here is a published
limitation, not a silent regression from what this capability's earlier
draft claimed.

#### Scenario: Default network posture
- **WHEN** `network.default = "deny"` and no `[network]` allowlist grants
  egress
- **THEN** the sandbox runs with `--network=none` and has no network
  connectivity of any kind, inbound or outbound

#### Scenario: Egress granted by the manifest
- **WHEN** the manifest's `[network]` section grants egress to specific
  hosts
- **THEN** the sandbox runs with `--network=host`, and Landlock applied
  to the Sentry process permits connections only to the granted hosts,
  denying everything else

#### Scenario: Egress still denied
- **WHEN** `network.default = "deny"` and a process inside the sandbox
  connects outward to a host or external address not in the allowlist
- **THEN** the connection is denied, and `why --host` explains it with
  the same origin vocabulary as the process tier

#### Scenario: Listen-socket gap persists at this tier
- **WHEN** `network.default = "deny"` and no `[network]` allowlist grants
  egress, and a process inside the sandbox binds `127.0.0.1:8080` and
  listens
- **THEN** the bind fails — because `--network=none` provides no network
  stack at all, not because of a targeted policy denial — so the
  outcome matches the process tier's tracked listen-socket gap (a dev
  server still cannot bind under deny-all) even though the underlying
  mechanism differs; this tier does not solve that gap

### Requirement: Sessions via native exec, dispatched host-side
The system SHALL run sessions (exec, shell, SSH-spawned) through the
backend's native exec-into primitive (`runsc exec`), with pty
allocation, signal forwarding, exit-code propagation, and the resolved
environment injection behaving identically to the process tier. No
keeper process runs inside the sandbox at this tier — the
listener-before-restriction fd-passing sequence is not used because
there is nothing to self-restrict; the SSH/control server instead runs
host-side, listening on the same 0600-socket-in-0700-dir pattern the
process tier uses (never TCP), and dispatches each session through
`runsc exec <container> -- <argv>` instead of a local fork/exec.

#### Scenario: Semantics match the process tier
- **WHEN** the same `devcroft exec`/`shell`/`ssh` workflows run against
  a hardened sandbox
- **THEN** observable behavior (cwd mapping, env, signals, exit codes,
  pty resize) matches the process tier exactly; only `status`'s tier
  line and performance differ

#### Scenario: No keeper inside the sandbox
- **WHEN** a hardened sandbox is running
- **THEN** no keeper process exists inside it; the control server runs
  on the host and every session is a discrete `runsc exec` invocation

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

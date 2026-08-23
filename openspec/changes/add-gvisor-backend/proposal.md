# Change: add-gvisor-backend

Status: in progress (a Linux host is now available via this repo's own
devcontainer; implemented directly against `runsc`, with `dragosv/mxc`'s
`gvizor` branch — a sibling project with a shipped gVisor backend — used
as an implementation *reference* for patterns, not as a dependency).
Depends on: `add-hardened-tier` (the tier abstraction this backend
plugs into, implemented alongside this change), `add-mvp-core` complete.

## Why

`add-hardened-tier` deliberately defines the hardened tier by its
guarantee, not its backend, and names two candidates without choosing.
That is the right shape for the tier spec — but half of its own open
questions (rootfs shape, gofer vs directfs, syscall overhead budget,
what `runsc exec` replaces) cannot be answered in the abstract; they are
properties of a concrete backend. This change concretizes the first one:
gVisor, the mature candidate — GKE-proven, well-understood failure
modes, and available today, unlike LiteBox, whose ABI-coverage and
process-model questions are still open research. The split mirrors the
structure that already worked for providers: `env-provider` defines the
contract, `add-nix-provider` delivered a concrete implementation against
it, and neither blocked the other.

**Corrected before implementation, from mxc's own real-world experience
building the same backend.** An earlier draft of this proposal led with
gVisor's per-sandbox netstack (`--network=sandbox`) closing the tracked
listen-socket gap for free — loopback binds under `deny`, two sandboxes
both binding `:3000` without conflict. That does not survive contact
with real `runsc`: `--network=sandbox` is rejected outright when
combined with `--rootless` ("sandbox network isn't supported with
--rootless"), and devcroft's whole design runs unprivileged by
construction (Landlock needs no privilege, nono drops root before
exec) — rootless is not a mode devcroft would trade away just for this.
So the delivered guarantee at this tier is the one that *does* hold
under rootless: Sentry services syscalls in user space, so an escape
needs a Sentry bug rather than a host kernel bug, and Landlock applied
to the Sentry process itself (defense in depth, see below) bounds a
compromised Sentry's filesystem reach by the same compiled policy. The
listen-socket / port-conflict gap tracked in docs/ssh-validation.md and
the README stays open at this tier too — published as a known
limitation, not silently dropped, matching this repo's own framing
rules. See the network policy bullet below for what *is* delivered.

## What Changes

- New backend adapter: `runsc`, resolved when `isolation = "hardened"`
  (the manifest key `add-hardened-tier` introduces; the manifest still
  never names a backend). `status` shows `isolation: hardened (gvisor)`.
- Platform selection: **systrap** by default (gVisor's default platform
  since mid-2023), **KVM** where `/dev/kvm` is accessible (bare metal).
  ptrace is legacy/deprecated upstream and is not targeted — this
  corrects `add-hardened-tier`'s "KVM or ptrace" phrasing, which
  predates systrap.
- Rootfs: synthesized at `up` from the provider's resolved closure — an
  OCI bundle whose rootfs is a minimal skeleton (`/etc`, `/tmp`, mount
  points) plus bind mounts: `/nix/store` read-only, the project root
  read-write, exactly the grants the compiled policy already names. No
  images, no registry, content-addressed and cheap — resolving
  `add-hardened-tier`'s "partially reintroduced images?" question with
  a concrete *no*.
- Store sharing: **directfs** (default in modern runsc) for the
  read-only store and project mounts; gofer as the compatibility
  fallback. Measuring which preserves the density advantage is a stated
  success criterion, not assumed.
- Policy compilation: the same `CompiledPolicy` (origins intact)
  projects to the OCI spec's mount list instead of a nono profile —
  note this model is deny-by-default (a path not mounted simply does
  not exist in the sandbox), which is *stronger* than the process
  tier's allow-then-deny — plus a Landlock profile applied to the
  Sentry process itself as defense in depth, exactly as
  `add-hardened-tier` sketches. `policy --render` output is identical
  across tiers.
- Network policy: no netstack (see above — incompatible with rootless).
  Default `--network=none` (no connectivity at all), matching
  `network.default = "deny"` with no allowlist. When the manifest's
  `[network]` section grants egress, the sandbox runs `--network=host`
  (gVisor's hostinet passthrough — a real network mode rootless mode
  accepts) with the same enforcement mechanism the process tier already
  uses: Landlock's TCP bind/connect restrictions, applied to the Sentry
  process, honoring the same `[network]` origins and answered through
  `why --host` with identical vocabulary. This reuses the existing
  `[network]` semantics rather than inventing netstack-derived ones; it
  does not close the listen-socket/port-conflict gap, since `--network=
  host` shares the host's network namespace exactly as the process tier
  does today.
- Sessions and SSH: `runsc exec` is the native exec-into primitive
  `add-hardened-tier`'s lifecycle delta anticipates — the
  listener-before-restriction fd-passing trick is unnecessary at this
  tier because a real exec-into-sandbox API exists, so no keeper runs
  inside the sandbox. Instead the SSH/control server runs host-side and
  dispatches every session (`exec`, `shell`, SSH-spawned) through
  `runsc exec <container> -- <argv>`. The boundary argument still holds:
  the unix socket's filesystem permissions (0600 socket, 0700 state
  dir, never TCP) were always the real access control, not the
  process's physical location — only where the *listener* runs changes.
  Session semantics (exec, shell, pty, signals, exit codes) remain
  identical to the process tier from the user's perspective.
- `doctor`: gVisor-specific probes — `runsc` presence and version
  range, platform availability (systrap kernel support, `/dev/kvm`
  accessibility for KVM), with each failure naming its fix and macOS
  reported as a permanent platform limitation, not a missing binary.

## Capabilities

### New Capabilities

- `gvisor-backend`: the runsc adapter — platform selection, rootfs
  synthesis from the provider closure, store sharing, policy
  projection, network policy enforcement, session execution via
  host-side `runsc exec` dispatch. Kept as its own
  capability so `add-hardened-tier`'s backend-generic deltas (tier
  selection in `config`, tier-conditional keeper in `lifecycle`,
  policy-target in `policy`) stay untouched and the two changes never
  collide at archive time.

### Modified Capabilities

- `cli`: `doctor` gains gVisor-specific diagnostics (additive
  requirement, distinct from `add-hardened-tier`'s generic
  "hardened-tier availability" one).

## Impact

- Affected specs: new `gvisor-backend`; `cli` (doctor). Deliberately
  *not* touching `config`/`lifecycle`/`policy` — those deltas belong to
  `add-hardened-tier` and stay backend-generic.
- Affected code (when implemented): a new backend module alongside the
  nono path; `up`'s backend dispatch; `doctor`; no manifest schema
  changes beyond what `add-hardened-tier` already adds.
- docs: the ssh-validation.md listen-socket gap and the README's
  port-conflict story stay open at this tier too, not just the process
  tier — the gap gets an honest tier-qualified answer ("not solved
  here either, and here's why"), not a false all-clear.

## Success Criteria

- On a supported Linux host, `isolation = "hardened"` resolves to
  gVisor, `up` succeeds, and the same manifest/sessions/SSH workflow
  from the process tier works unchanged, dispatched through `runsc
  exec` instead of the keeper's local spawn.
- Filesystem access is deny-by-default via the OCI mount model: a path
  neither granted by the manifest nor part of the provider's store
  grants nor the baseline skeleton is absent inside the sandbox, not
  present-but-denied.
- The Sentry process itself runs under a Landlock profile compiled from
  the same policy, so a compromised Sentry's filesystem reach stays
  bounded by the same grants the sandbox has.
- `policy --render` on the same manifest is byte-identical across
  tiers; `why` answers identically, including for `[network]` origins.
- Egress is denied under `network.default = "deny"` and permitted per
  the manifest's allowlist when granted — matching the process tier's
  behavior and vocabulary. (The listen-socket/port-conflict gap is
  explicitly *not* closed at this tier; see the network policy bullet
  above.)
- Published benchmarks (per `add-hardened-tier`'s criterion): a
  mid-size `cargo build` and an `npm ci` across process tier, gVisor
  systrap, and gVisor KVM, with directfs on and off — real numbers for
  the syscall-overhead question, not folklore.

## Open Questions

- **runsc version policy.** gVisor ships continuously, not semver;
  decide what "tested range" means for `doctor` (pin a release tag
  range? a minimum release date?).
- **Non-rootless netstack, as a future extension.** A hardened backend
  running non-rootless could use `--network=sandbox` and deliver the
  original netstack story: loopback binds under `deny`, no port conflicts
  between sandboxes. Worth recording as a real, considered option for a
  later change — not chosen now, since it trades away the unprivileged
  posture every other tier holds to, for a single tier's network story.

  **Cost measured 2026-08-23, and it is higher than this entry first
  assumed.** The original wording proposed a scoped, NOPASSWD-limited
  privilege grant, mirroring the narrow sudo rule this repo gives flox's
  nix-daemon. That shape is now known to be insufficient: running `runsc`
  as root clears the userns/`newuidmap` requirement but then fails on
  `CAP_SYS_ADMIN`, so the grant is root *plus* `CAP_SYS_ADMIN` in the
  container's bounding set. `newuidmap` is also a requirement of any
  non-rootless run by an unprivileged user, independent of network mode.
  Full matrix and what was ruled out: `docs/decisions.md`, the netstack
  rejection entry.
- **Provisioning stays host-side.** Provider resolution (`nix develop`,
  `flox activate`) runs before restriction, on the host, unchanged —
  but the captured env diff now describes paths that must all be
  reachable through the bundle's mounts. Confirm the store-grant set
  the providers emit is exactly the mount set the bundle needs, or `up`
  must fail naming the gap (the existing "provider resolution must not
  widen the policy" invariant, projected onto mounts).
- **cgroup limits.** runsc integrates with cgroups; devcroft's MVP
  explicitly punted on resource limits. Adding them here would be
  scope creep — but the door opens, so the rejection in
  docs/decisions.md may need a "revisit at hardened tier" note.

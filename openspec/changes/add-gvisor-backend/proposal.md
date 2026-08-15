# Change: add-gvisor-backend

Status: proposed (post-MVP sketch — proposal + delta specs, no tasks.md,
same convention as `add-mise-provider` and `add-hardened-tier`; tasks
come when there is a Linux host/CI to implement and validate against,
since gVisor is Linux-only and this repo currently develops on macOS).
Depends on: `add-hardened-tier` (the tier abstraction this backend
plugs into), `add-mvp-core` complete.

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

One consequence deserves top billing because it fixes a tracked gap
rather than adding a feature: **gVisor gives every sandbox its own
user-space network stack (netstack)**. Inside the sandbox, binding
loopback touches nothing on the host — so "my dev server can listen,
but has no egress", the single most common shape of local development
and currently impossible to express (docs/ssh-validation.md's
highest-priority finding: under the default policy *no* dev server can
bind a port, which broke VS Code Remote-SSH and the Gin sample alike),
falls out of the architecture for free at this tier. Two sandboxes both
binding :3000 stop being a port conflict at all — each has its own
netstack — which finally makes the README's port-conflict story true
instead of optimistic.

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
- Network policy: netstack makes the `[network]` section *more*
  enforceable, not less — loopback binds inside the sandbox are local
  to it (allowed even under `default = "deny"`, closing the tracked
  listen-socket gap at this tier), egress is mediated and blockable
  per-sandbox, and reaching an inside listener from the host goes
  through explicit port forwarding rather than shared host ports.
- Sessions: `runsc exec` is the native exec-into primitive
  `add-hardened-tier`'s lifecycle delta anticipates — the
  listener-before-restriction fd-passing trick is unnecessary at this
  tier because a real exec-into-sandbox API exists. Session semantics
  (exec, shell, pty, signals, exit codes) remain identical to the
  process tier.
- `doctor`: gVisor-specific probes — `runsc` presence and version
  range, platform availability (systrap kernel support, `/dev/kvm`
  accessibility for KVM), with each failure naming its fix and macOS
  reported as a permanent platform limitation, not a missing binary.

## Capabilities

### New Capabilities

- `gvisor-backend`: the runsc adapter — platform selection, rootfs
  synthesis from the provider closure, store sharing, policy
  projection, netstack semantics, session execution. Kept as its own
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
- docs: the ssh-validation.md listen-socket gap gets a
  tier-qualified answer; README's port-conflict story becomes accurate
  for the hardened tier.

## Success Criteria

- On a supported Linux host, `isolation = "hardened"` resolves to
  gVisor, `up` succeeds, and the same manifest/sessions/SSH workflow
  from the process tier works unchanged.
- A dev server binds loopback inside the sandbox under
  `network.default = "deny"` and is reachable from the host via the
  forwarded port — the exact scenario that fails at the process tier
  today.
- `policy --render` on the same manifest is byte-identical across
  tiers; `why` answers identically.
- Two sandboxes bind the same port simultaneously without conflict.
- Published benchmarks (per `add-hardened-tier`'s criterion): a
  mid-size `cargo build` and an `npm ci` across process tier, gVisor
  systrap, and gVisor KVM, with directfs on and off — real numbers for
  the syscall-overhead question, not folklore.

## Open Questions

- **Where the SSH server lives.** The keeper currently *is* the SSH
  endpoint, and the ssh capability's invariant ("SSH lives inside the
  boundary") assumed the process tier's architecture. With `runsc exec`
  available, the server could run host-side using exec-into as its
  session backend (the boundary story holds — sessions still run
  inside; the unix socket's filesystem permissions were always the real
  access control), or the keeper could simply keep running inside the
  sandbox for SSH alone, forfeiting the keeper-less simplification.
  Behaviorally both must look identical through `devcroft proxy`; which
  to build is an implementation decision this sketch leaves open.
- **runsc version policy.** gVisor ships continuously, not semver;
  decide what "tested range" means for `doctor` (pin a release tag
  range? a minimum release date?).
- **Port forwarding UX.** Netstack means an inside listener needs
  explicit forwarding to be reachable from the host. Decide the
  manifest/CLI surface for it (a `[network] forward` list? a
  `devcroft forward` command? automatic for `-L`-style SSH forwarding,
  which already works through the existing channel support?).
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

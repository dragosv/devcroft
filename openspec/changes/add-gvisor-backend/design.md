# Design: gVisor backend (`runsc`)

## Context

See proposal.md — Why, for motivation, and `add-hardened-tier`'s
design.md for the tier-generic seams this backend plugs into
(`SessionBackend`, tier dispatch in `up`, the tier-conditional SSH
placement).

`runsc` is an OCI runtime: unlike `nono`, which takes policy as a flag
and a profile file, it consumes an **OCI bundle** — a directory holding a
runtime `config.json` and a `rootfs/`. So this backend has a
materialization step the process tier has no analogue for.

**Provenance.** `dragosv/mxc`'s `gvizor` branch ships a working gVisor
backend for a different product (one-shot script sandboxing). It was read
as a reference for `runsc` mechanics — bundle shape, argument assembly,
teardown ordering — and its real implementation experience corrected a
false premise in this change's own earlier draft (decision 1). Nothing is
taken as a dependency: mxc's policy model, config schema, and execution
model are not devcroft's, and this backend compiles `CompiledPolicy`
directly.

## Goals / Non-Goals

**Goals:**

- Long-lived sandboxes (`up`/`down`/sessions), not one-shot execution.
- Deny-by-default filesystem via the OCI mount model, compiled from the
  same `CompiledPolicy` the process tier uses, origins intact.
- Rootless by default, matching devcroft's unprivileged posture
  everywhere else.

**Non-Goals:**

- Per-sandbox netstack, and therefore the listen-socket/port-conflict
  fix an earlier draft promised (decision 1).
- cgroup resource limits. `runsc` supports them and MVP punted on them
  deliberately; adding them here is scope creep. The door opening is
  noted in the proposal.
- KVM validation in CI. Systrap is the default and the only platform
  exercised automatically; KVM is selected when usable but best-effort.

## Decisions

### 1. No netstack: `--network=none` / `--network=host`, not `--network=sandbox`

**This reverses this change's own earlier draft, which led with netstack
as the headline win.** `runsc` rejects `--network=sandbox` together with
`--rootless` outright ("sandbox network isn't supported with
--rootless") — found by mxc running it against real `runsc`, not
inferred from docs. It is a runtime-level restriction, not a
configuration detail to work around.

Rootless is not negotiable here: every other part of devcroft is
unprivileged by construction (creating a Landlock ruleset needs no
privilege; nono drops root before exec; the devcontainer runs as
`vscode`). Trading that away for one tier's network story would make the
hardened tier the *only* component demanding privilege, which is a worse
posture than the gap it closes.

So: `--network=none` when the policy grants no egress, `--network=host`
(hostinet passthrough) when it does, with `[network]` enforcement by the
same Landlock TCP restrictions and the same origin vocabulary the
process tier already uses.

Consequence, stated plainly because this repo's framing rules require
it: the listen-socket / port-conflict gap tracked in
docs/ssh-validation.md and the README is **not** closed at this tier.
It is published as a known limitation, not quietly dropped.

Alternative considered: run non-rootless behind a narrowly scoped
NOPASSWD sudo rule (the pattern this repo already uses for flox's
nix-daemon) to unlock netstack. Rejected *for now* and recorded in the
proposal's Open Questions as a real future option — it is a coherent
design, just not one worth taking before the tier's core guarantee
ships.

### 2. Bundle is persistent, per sandbox — not per execution

mxc materializes a bundle per execution in `$XDG_RUNTIME_DIR` and
deletes it on completion, which is right for one-shot scripts. devcroft's
sandboxes are long-lived: `up` starts one, many sessions attach to it,
`down` stops it. So the bundle lives under the existing
`<state>/<name>/` directory, is built once at `up`, rebuilt by
`up --recreate`, and removed by `rm` — the same lifecycle every other
piece of sandbox state already follows.

### 3. `CompiledPolicy` projects onto mounts; no parallel policy model

The bundle's mount list is a *projection* of `CompiledPolicy`, exactly as
`to_nono_profile()` is. Rule origins are untouched, so `policy --render`
and `why` produce byte-identical output regardless of tier — which is a
stated success criterion and is preserved by construction (see
`add-hardened-tier` design decision 4), not by re-implementing rendering
per backend.

The mount model is deny-by-default: a path not mounted does not exist
inside the sandbox, rather than existing-but-denied. This is *stronger*
than the process tier's allow-then-deny, and the spec says so.

### 4. Landlock on the Sentry process is new code, not reuse

Nothing in `src/` applies Landlock directly today — the process tier's
enforcement lives entirely inside the external `nono` binary. Confining
the Sentry (defense in depth: gVisor already seccomps it; Landlock
bounds its filesystem reach by the same compiled policy) therefore needs
a new `landlock` crate dependency and a from-scratch application step.
Flagged explicitly because it is the one part of this change with no
existing in-repo precedent to copy.

### 5. Sessions via `runsc exec`, dispatched from a host-side server

`runsc exec` is the native exec-into primitive `add-hardened-tier`'s
lifecycle delta anticipates, so this backend implements `SessionBackend`
over it and no keeper runs inside the sandbox. Teardown must go through
`runsc kill` + `runsc delete` rather than killing the client process —
the sandbox process tree is separate, a lesson mxc's implementation
records explicitly — with process-group kill only as a fallback.

### 6. Platform: systrap default, KVM when actually usable

Systrap is `runsc`'s default since mid-2023 and needs no special host
access. KVM is selected only when `/dev/kvm` is present *and* accessible
to the invoking user — presence alone is not enough, and probing it is
`doctor`'s job. ptrace is deprecated upstream and not targeted, which
corrects `add-hardened-tier`'s original "KVM or ptrace" phrasing.

## Risks / Trade-offs

- **Syscall compatibility**: gVisor implements most, not all, of Linux;
  an exotic toolchain may fail inside gVisor while working at the process
  tier → Documented as a tier caveat; `runsc`'s debug logging is the
  diagnosis path.
- **Build performance**: builds are syscall-heavy and systrap adds
  per-crossing cost, plus bundle materialization at `up` → This is the
  tier's known cost, which is why `hardened` is opt-in and why the
  proposal requires published benchmarks rather than folklore.
- **Rootless needs unprivileged user namespaces**, which some hosts
  disable → `doctor` must probe the platform rather than infer from
  binary presence, and name the fix. This devcontainer is currently such
  a host (`unshare --user` → EPERM), which is what task group 8 addresses.
- **A false promise was already published in this change's own draft** →
  Corrected in-place in the proposal and spec rather than silently
  edited around, and the reversal is recorded here (decision 1) so the
  reasoning survives.

## Migration Plan

Purely additive and opt-in: reachable only via `[sandbox].isolation =
"hardened"`, which defaults to `process`. Rollback is dropping the
`src/gvisor` module and the hardened dispatch arm. No manifest schema
change beyond the key `add-hardened-tier` already adds.

## Open Questions

- **runsc version policy.** gVisor ships continuously, not semver;
  what "tested range" means for `doctor` (a pinned tag range? a minimum
  release date?) can be decided once CI pins one. Safely deferrable: it
  changes a diagnostic message, not the specs or the approach.
- **directfs vs. gofer for store sharing.** directfs is the modern
  default and is what this ships with; whether gofer is ever needed as a
  compatibility fallback is a measurement question the proposal's
  benchmark criterion already covers.

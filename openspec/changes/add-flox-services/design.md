# Design: add-flox-services

## Context

See `proposal.md` — Why, and its Blocking Dependency section, which
gates implementation entirely.

Three existing constraints shape everything below:

1. **Two-phase execution** (CLAUDE.md): provisioning is host-side and
   trusted; everything after restriction is project code. Services are
   project code.
2. **Environment resolves once, at `up`** — and the invariant's own
   stated consequence: per-session provider activation is rejected
   because "it would force the profile to grant flox internals forever."
3. **`SessionBackend`** already exists as a seam (added by
   `add-hardened-tier`): `spawn(&SpawnRequest) -> SpawnedSession`, with
   a local fork/exec implementation and a `runsc exec` implementation
   behind the same trait.

## Goals / Non-Goals

**Goals:**

- Services supervised with per-service state, at both isolation tiers,
  without a tier-specific code path.
- No flox binary, and no provider internals of any kind, reachable
  inside the sandbox.
- Failure visible by default; nothing silently restarted into looking
  healthy.

**Non-Goals:**

- Per-sandbox port allocation (see Decisions — deliberately deferred).
- A `devcroft services` command. Service state rides on the existing
  `status`/`ps`/`logs`.
- Readiness/health checks, dependency ordering between services, or any
  new key in the flox manifest — devcroft does not own that schema.

## Decisions

### 1. Start each service's command directly; never run `flox services` inside the sandbox

The obvious implementation — shell out to `flox services start` — is
rejected, and not merely on taste. `flox services start` would have to
run inside the boundary (services are project code), which requires the
flox binary and its internals to be executable inside the compiled
profile. That is the exact thing the "environment resolves once"
invariant already rejects for per-session activation, for the same
reason: the profile would have to grant flox internals permanently, for
every sandbox, forever.

Instead: service **declarations** are read host-side during resolution
(the trusted phase, alongside env capture), and each declared service's
`command` is started as a supervised child **inside** the sandbox, with
the already-captured environment. flox's own supervisor is never
involved at runtime.

Secondary benefit: this avoids two supervisors fighting. flox restarts
what it supervises; devcroft must be able to reap deterministically at
`down`. Only one of them can own process lifetime, and it has to be the
one that owns the sandbox.

Cost, stated honestly: `flox services status` run by hand inside a
project will not show these processes, because flox did not start them.
The declaration is shared; the supervision is not.

### 2. Services are `SessionBackend` spawns without a pty

A service is a non-interactive process, in the sandbox, that nobody is
attached to. That is a session minus the pty and minus the client. So
services go through the existing `SessionBackend` trait rather than a
parallel spawn path.

This is the decision that makes services tier-agnostic for free: at the
`process` tier they fork/exec under the applied profile; at the
`hardened` tier the identical code dispatches through `runsc exec` into
the gVisor sandbox. No tier-specific service logic, and no risk of the
two tiers diverging in service behavior — the same property
`add-hardened-tier` bought for sessions.

Alternative considered: a dedicated supervisor process per sandbox.
Rejected — it adds a second resident process to a design whose keeper is
already documented as a single point of failure, and buys nothing the
keeper's existing registry does not already do for sessions.

### 3. No automatic restart in the first cut

A crashed service is reported as failed and left dead. Auto-restart is
deliberately not implemented.

Rationale specific to the target user: for a fleet of coding agents, a
crash-looping Postgres is strictly worse than a visibly dead one. An
agent that sees "service failed, here is the log tail" can act; an agent
watching a service flap sees intermittent connection errors and will
usually misdiagnose them as its own bug — precisely the "agent debugs
healthy code in a broken environment" failure the README's verification
bullet warns about.

Revisitable: an explicit restart policy key is additive later. It should
not be inferred by default.

### 4. Ordering — services after hooks, and `--skip-hooks` skips both

Both orderings have real use cases (a hook seeding fixtures a service
needs; a service a hook wants to query). Hooks-first is chosen because
hooks already have "fails `up`" semantics and services explicitly do
not — running the failure-significant step first keeps `up`'s failure
mode simple.

`--skip-hooks` also skipping services is not an obvious reading of the
flag's name, but it preserves the property that actually matters: one
flag that guarantees nothing project-supplied executes. Splitting that
into two flags would leave users who reach for `--skip-hooks` to debug a
broken environment still running project code.

### 5. Port allocation deferred, not designed around

N sandboxes declaring Postgres on 5432 collide. This is not fixable
within this change: there is no network namespace separation between
sandboxes (`add-mvp-core` design.md Decision 5), and the hardened tier
does not add one either — `runsc` rejects `--network=sandbox` under
`--rootless`, and devcroft is unprivileged everywhere by design
(`docs/decisions.md`, "Rejected (for now): non-rootless gVisor for
netstack").

Rather than inventing a port-rewriting layer that would have to
translate ports inside service commands devcroft does not own, the
limitation is published. The honest fix is either the scoped-privilege
netstack option that decision already records as revisitable, or a
devcroft-owned port allocation contract — both larger than this change.

### 6. Declarations are part of the existing staleness fingerprint

flox's `manifest.toml` is already fingerprinted whole, so editing
`[services]` flips `status` to stale and requires `up --recreate`. This
is accepted rather than special-cased: carving services out of the
fingerprint would mean a sandbox whose declared services no longer match
what is running, reported as fresh. Heavier than users may expect;
correct.

## Risks / Trade-offs

- **The listening-socket gap makes this untestable end to end** →
  Mitigation: none available within this change. This is why the
  proposal states it as a blocking dependency rather than a caveat.
  Integration tests must be written against a policy that permits
  binding, and must be honest that they exercise a configuration the
  default policy does not allow.

- **flox `[services]` schema drift** → Mitigation: read declarations
  through flox's own machine-readable output if one exists, falling back
  to parsing the manifest, and pin the tested flox range the way
  `doctor` already pins nono and nix. A schema change should fail loudly
  at `up`, not produce a silently empty service list.

- **Split-brain with `flox services`** (Decision 1's stated cost) →
  Mitigation: document it, and have `doctor` or `status` name devcroft
  as the supervisor so a user running `flox services status` by hand is
  not confused by an empty list.

- **Keeper remains a single point of failure, now for services too** →
  Mitigation: none new; this widens an already-published gap rather than
  introducing one. Worth restating in the README's known gaps so the
  blast radius of a dead keeper is not understated.

- **A service that exits instantly looks the same as one that never
  started** → Mitigation: distinguish not-started, failed-at-start, and
  exited-later in service state, as the `services` spec requires.

## Migration Plan

Additive and inert by default: a project declaring no services behaves
exactly as today, and the feature has no user-visible surface until a
`[services]` section exists. No manifest migration, no state-format
break for existing sandboxes beyond whatever field records service
state, which reads as empty for sandboxes that predate it (the same
posture `add-hardened-tier` used for the isolation-tier field).

Rollback is removal: services stop being started, and nothing else
changes.

## Open Questions

- Whether flox exposes a machine-readable listing of *declared* (not
  running) services. If it does, use it; if not, parse `[services]` from
  the manifest. Deferrable because it changes neither the specs, the
  approach, nor the task breakdown — only which call the declaration
  reader makes internally.
- Whether service log output should share the keeper log or get a
  per-service file. Deferrable: the `cli` spec requires attribution, not
  a particular storage layout.

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

### 1. Read the documented declarations; generate our own process-compose config

Three implementations were considered, and the first two were both
rejected after being investigated live rather than reasoned about.

**(a) Shell out to `flox services start`.** Rejected: it would have to
run inside the boundary (services are project code), requiring the flox
binary and its internals to be executable inside the compiled profile.
That is what the "environment resolves once" invariant already rejects
for per-session activation, for the same reason — the profile would grant
flox internals permanently, for every sandbox.

**(b) Consume flox's generated `service-config.yaml` directly.** This
looked strictly better at first, and it is worth recording why it is not.
Investigated live: flox does use process-compose internally, invoking
`process-compose up -f $FLOX_ENV/service-config.yaml -u <sock>`, and that
config is readable from inside a devcroft sandbox today. Two facts killed
it:

- `service-config.yaml` is an **undocumented generated artifact**. It
  appears in none of flox's published documentation, unlike `[services]`
  in `manifest.toml`, which is a documented user-facing schema. Consuming
  it would trade a public contract for an implementation detail — and one
  whose contents (`flox_never_exit` with `sleep infinity`, flox's own
  keep-alive) are visibly tailored to flox's lifecycle rather than to
  third-party consumption.
- The process-compose binary it needs is **flox's own dependency, not the
  environment's** — confirmed: zero of the environment closure's 29
  requisites, with `flox-1.14.0` itself as the referrer. It is readable
  from a sandbox only because devcroft grants `/nix/store` broadly rather
  than granting the environment's actual closure. That makes the whole
  approach work by accident, and it would break the day those grants are
  tightened — which would be an improvement devcroft should be free to
  make.

**(c) Chosen: parse the documented `[services]` declarations host-side,
generate a process-compose config devcroft owns, and run process-compose
supervised by the keeper inside the sandbox.**

This keeps the stable half of (b) — no reimplementation of restart
policy, service dependencies, or daemon handling, all of which
process-compose already does — while depending only on a published
schema. The generated config is devcroft's own artifact, so nothing
breaks when flox changes its internal one. Because every provider's
declarations land in the same internal model, one config generator serves
flox, devbox (which is process-compose-based too), and nix-with-
services-flake — the whole provider roadmap, not just today's provider.

`process-compose` must therefore be **declared in the project's
environment**, so it is a real closure member rather than a scanned store
path. Scanning is rejected: it picks an arbitrary path with nothing tying
it to this environment's config schema, and it happens to work today only
because exactly one copy exists on this machine.

Cost, stated honestly: `flox services status` run by hand will not show
these processes, because flox did not start them. The declarations are
shared; the supervision is not. And requiring `process-compose` in the
manifest leaks a devcroft implementation choice into the project's
environment — accepted as the lesser evil against depending on a binary
the environment never declared.

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

### 4. Ordering — services BEFORE hooks (reversed), and `--skip-hooks` skips both

**This decision originally said hooks first. That was wrong, and the
reason it was wrong is worth keeping.** The original argument was that
hooks have "fails `up`" semantics and services do not, so running the
failure-significant step first keeps `up`'s failure mode simple. That is
an argument about devcroft's internals, not about what projects actually
need.

The real-world dependency runs the other way: the canonical `post_start`
hook for a project with a database is "run migrations", which requires
the database to already be up. Services-then-hooks serves that; the
reverse forecloses it.

It is also what the implementation forces, which is how the error
surfaced. `up` cannot own service lifetime — it is a short-lived CLI
process, and a session whose client disconnects is escalated after
`connection::DEFAULT_GRACE_PERIOD` (2s), so services started the way
hooks are started would die ~2 seconds after `up` returns. The keeper
must own them, and the keeper's natural moment is its own startup —
before `up` gets far enough to run hooks. Constraint and correctness
agree here; the original decision had them in conflict only because it
was reasoned from the wrong end.

The case this forecloses — a service that wants a hook-seeded fixture
before it starts — is real but rarer, and is served by the service's own
command waiting, not by reordering the whole phase.

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

### 7. Per-service state comes from process-compose's API, not from devcroft's own bookkeeping

Decision 2 makes process-compose a single registry entry, which is right
for teardown and wrong for reporting: the `services` spec requires
distinguishing not-started, failed-at-start, running and exited-later,
and one entry for the whole group cannot express any of that. A sandbox
whose database died at startup currently looks exactly like one whose
database is serving traffic.

That gap is not cosmetic, and it undermines decision 3 specifically.
Auto-restart was rejected on the grounds that "an agent that sees
'service failed, here is the log tail' can act" — an argument that
assumes a visibility devcroft does not yet provide. Either this is built
or decision 3 loses its justification.

**Chosen: query process-compose over the unix socket it already
listens on.** That socket exists for this reason — decision 2 chose
`-u <socket>` over `--no-server` precisely to keep the API reachable —
so nothing new has to be started or plumbed. Verified live from inside a
sandbox that `process-compose process list -u <socket> -o json` returns
per-service `status`, `exit_code`, `is_running`, `pid`, `restarts` and
`age`, which covers every state the spec asks for.

Alternatives considered:

- **Parse `.devcroft/services.log`.** Rejected: it is a human-readable
  log with no stability promise, and reconstructing state from log lines
  reintroduces exactly the "depend on an undocumented internal" mistake
  decision 1 already corrected once.
- **Track state in devcroft by supervising each service directly.**
  Rejected for the reason decision 1 gives: it means reimplementing
  restart policy, dependencies and daemon handling, and it would put
  devcroft's view and process-compose's view in permanent disagreement.

Two implementation facts found while probing, worth carrying:

- The CLI writes warn/debug lines to stdout ahead of the JSON (a failed
  `getpwuid` for uid 1000, and a missing XDG config dir — both harmless
  inside a sandbox). Output must be parsed from the first `[`, not
  assumed to be clean JSON, or the first parse attempt fails on noise.
- Not yet confirmed: the exact `status` strings for a service that fails
  at startup versus one that exits later. The fields exist; the mapping
  from them to devcroft's four states must be established against real
  failures during implementation, not assumed from the running case.

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

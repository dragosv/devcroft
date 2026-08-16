# Design: add-hardened-tier

## Context

See proposal.md — Why, for motivation and the backend candidate analysis.

Constraints that shape this design, all from the existing MVP code:

- `policy::compile` → `CompiledPolicy` (src/policy/mod.rs) is already
  backend-agnostic. `to_nono_profile()` is the *only* backend-specific
  projection, and `render.rs`/`why.rs` both operate on `CompiledPolicy`
  *before* any projection. So "`policy --render` is identical across
  tiers" is already true by construction; this change must be careful
  not to break that, not to build it.
- `keeper::session::spawn()` was a free function doing local fork/exec,
  called from two places: `keeper::connection` (control socket) and
  `ssh::server` (SSH channels). Both needed to become backend-dispatched.
- `lifecycle::up` hardcodes the nono path: compile → `to_nono_profile()`
  → create listeners → spawn keeper under `nono wrap` with inherited fds.
  The listener-before-restriction ordering is load-bearing *for that
  tier specifically* (CLAUDE.md), because Landlock/Seatbelt can only be
  applied by a process to itself and its descendants.

## Goals / Non-Goals

**Goals:**

- One manifest key (`[sandbox].isolation`) selecting an intent, resolved
  per host, with `process` behavior bit-for-bit unchanged.
- A seam that lets a hardened backend supply session execution without
  the session/protocol/pty/registry layers knowing which backend is in
  play.
- Keep every tier-generic decision here, so a concrete backend change
  (`add-gvisor-backend`) never has to touch `config`/`lifecycle`/
  `policy`/`ssh` and the two changes cannot collide at archive time.

**Non-Goals:**

- Any concrete backend. This change names no runtime; `add-gvisor-backend`
  supplies the first one.
- Changing `process`-tier behavior in any observable way. The refactor
  below is behavior-preserving by construction and is verified by the
  existing keeper/ssh/exec tests passing unmodified.

## Decisions

### 1. `SessionBackend` trait, not a tier enum threaded through the keeper

Session execution differs by tier; everything around it (framing, pty
allocation, signal forwarding, exit-code propagation, the registry) does
not. Making the *spawn step* polymorphic — `trait SessionBackend { fn
spawn(&self, req: &SpawnRequest) -> io::Result<SpawnedSession> }` — keeps
that difference in exactly one place.

`LocalSessionBackend` holds today's fork/exec body verbatim, so the
`process` tier is unchanged. `Keeper::new` and `ssh::server::spawn` take
an `Arc<dyn SessionBackend>`.

Alternatives considered:

- *Tier enum branch inside `session::spawn`.* Rejected: it would force
  every hardened backend's process-handling into `keeper/`, which is the
  process tier's module, and would make `keeper` depend on backend
  modules rather than the reverse.
- *Generic parameter `Keeper<B: SessionBackend>`.* Rejected: it
  infects `connection::handle`'s signature and the ssh server's handler
  struct with a type parameter for no benefit — dispatch happens once
  per session, so the vtable cost is irrelevant next to a process spawn.

The decisive detail found while implementing: `ssh/server.rs` called
`session::spawn` directly as a *second* call site. Without routing it
through the same trait, a hardened sandbox would have dispatched
`exec`/`shell` correctly but silently fork/exec'd SSH sessions on the
host — outside the sandbox entirely. Both call sites go through the
trait.

### 2. The keeper is tier-conditional; the SSH *socket contract* is not

Where the SSH server runs is tier-dependent (see specs/ssh/spec.md);
what guards it is not. `add-mvp-core`'s ssh spec conflated the two, so
this change's `ssh` delta separates them explicitly rather than leaving
a contradiction between the spec and the hardened tier's architecture.

For a hardened backend with a native exec-into primitive there is
nothing to self-restrict host-side, so the listener-before-restriction
fd-passing sequence is not used and no keeper runs inside the sandbox.
The host-side control process is not the trust boundary at that tier —
the backend's own sandboxing is.

Alternative considered: keep a keeper inside the sandbox for SSH alone.
Rejected in favor of the above (resolving the proposal's open question),
because it forfeits the simplification the native primitive exists to
provide while still paying for a resident process; behaviorally the two
are indistinguishable through `devcroft proxy`, so the simpler
architecture wins.

### 3. Tier resolution fails loudly, never downgrades

`hardened` on a host that cannot provide it is a hard failure at layer
`backend` (error contract, CLAUDE.md), never a silent fall back to
`process`. This is the same posture as `provider` rejection: a user who
asked for a security boundary must never be silently given accident
protection instead. On macOS the failure names the platform limitation
as permanent rather than suggesting an install.

### 4. Policy stays one compilation with two projections

`CompiledPolicy` and its origins are tier-independent; only the final
projection differs (`to_nono_profile()` vs. a hardened backend's own).
`render`/`why` are deliberately left untouched, since they read
`CompiledPolicy` directly — the tier-independence they need is a
property of where they sit in the pipeline, and a regression test
asserts it rather than trusting it.

## Risks / Trade-offs

- **The `SessionBackend` refactor touches the process tier's hot path** →
  Mitigated by keeping `LocalSessionBackend`'s body byte-identical to the
  previous free function and requiring the existing keeper/ssh/exec test
  suites to pass unmodified. They do (155/155 lib tests).
- **Two SSH server placements to keep behaviorally identical** → The ssh
  delta's "Client cannot tell the difference" scenario is the contract;
  it needs a real cross-tier test, not just review.
- **Tier-conditional code paths double the lifecycle surface** →
  Accepted, and bounded: the split is at one dispatch point in `up`, not
  scattered through the session layers, precisely because of decision 1.

## Migration Plan

Purely additive. `[sandbox].isolation` defaults to `process`, so every
existing manifest keeps its exact current behavior with no edit. Rollback
is removing the key and the hardened dispatch arm; `LocalSessionBackend`
would simply become the only implementation again.

## Open Questions

- **Whether `hardened` should ever be a per-host default for any workload
  class.** Deferred safely: it changes no spec and no task here, only a
  future default. Needs the benchmarks the proposal's success criteria
  already require.

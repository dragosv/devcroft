# Change: add-port-allocation

Status: proposed (post-MVP). Depends on: `add-flox-services` (services
are where the collision actually bites) and `network.ports` (already
implemented — the mechanism that grants a loopback port at all).

## Why

devcroft's stated audience is fleets of parallel environments on one
host, and the motivating example for services is "each agent gets its
own Postgres instead of sharing the host's". That does not work today at
N > 1, which is the only N that matters for the claim.

The reason is mundane and structural. A project declares its port in
`devcroft.toml`, that file is committed, and every git worktree of the
repo therefore declares the *same* port. Two sandboxes both asking for
5432 collide with `EADDRINUSE`. There is no PID/mount/net namespace
separation between sandboxes (`add-mvp-core` design.md Decision 5), and
the hardened tier does not add one either — `runsc` rejects
`--network=sandbox` under `--rootless`, and devcroft is unprivileged
everywhere by design (`docs/decisions.md`).

So the fix cannot be isolation. It has to be allocation: distinct real
ports per sandbox, chosen by devcroft, and — the half that is easy to
forget — *discoverable*, since a port nobody can find is no more useful
than a port that collides.

This is also the second half of a problem whose first half is already
proposed elsewhere: `add-agent-workload` fixes N worktrees silently
sharing one sandbox *name*. Both have the same root cause — a committed
file describing an instance — and fixing only one leaves the fleet case
broken.

## What Changes

- **A manifest may ask devcroft to allocate a port rather than fix one.**
  Allocation is requested by naming the environment variable that should
  carry the result, not by naming a number.
- **The allocated port is injected into the sandbox environment**, and —
  for a declared service — into the generated process-compose config, so
  the service starts on the port devcroft actually granted.
- **The allocated port is granted in the compiled policy** exactly as a
  fixed `network.ports` entry is, with an origin distinguishing it as
  allocated rather than manifest-declared.
- **Allocation is sticky for the sandbox's life, not per `up`.** The
  chosen port is recorded, so a connection string a user or agent wrote
  down does not silently change under them on the next `up`.
- **`status` reports every allocated port with the variable carrying
  it.** Without this the feature is unusable: an agent that cannot
  discover its own database port is no better off than one whose
  database failed to bind.
- **A service whose port is hardcoded in its command string cannot be
  allocated**, and the manifest asking for both SHALL fail loudly rather
  than allocate a port nothing listens on. This follows from what
  devcroft owns: it generates the `environment` block of the
  process-compose config, so it can substitute a variable's value — it
  does not own, parse, or rewrite the command strings themselves.

## Capabilities

### New Capabilities

- `port-allocation`: choosing a free loopback port per sandbox, making
  it reachable to the processes that need it, keeping it stable for the
  sandbox's life, and surfacing it so it can be connected to.

### Modified Capabilities

- `config`: `[network]` gains the allocation request, validated with the
  same strictness as `ports`.
- `policy`: allocated ports appear in the compiled profile and in
  `policy --render` with an origin marking them allocated, so the
  "nothing reaches the backend that `--render` cannot show" invariant
  continues to hold for rules devcroft chose rather than the user.
- `services`: the generated process-compose config carries the allocated
  value, overriding what the provider's own `vars` declared.
- `cli`: `status` reports allocated ports and their variables.

## Impact

- Affected specs: new `port-allocation`; modified `config`, `policy`,
  `services`, `cli`.
- Affected code: `src/config/` (the request), `src/services/` (config
  generation, and the port chooser itself), `src/policy/` (compiled
  grants with a new origin), `src/lifecycle/` (allocate at `up`, record
  in `meta.json`, inject into the keeper environment),
  `src/bin/devcroft.rs` (`status`).
- Interacts with `add-agent-workload`: that change gives N worktrees
  distinct sandbox *names*; this one gives them distinct *ports*. Either
  alone leaves fan-out broken, and they should be evaluated together
  when judging whether the fleet claim holds.

## Success Criteria

- Two sandboxes from two worktrees of one repo, with the identical
  committed manifest, both come up with the same declared service and
  neither collides.
- Each reports its own port through `status`, and connecting to the
  reported port reaches that sandbox's service and not the other's.
- The port survives `down` then `up` for the same sandbox: a connection
  string stays valid for the sandbox's life.
- `policy --render` shows the allocated port with an origin identifying
  it as allocated, distinct from `manifest:network.ports`.
- A manifest requesting allocation for a service whose command hardcodes
  its port fails at `up`, naming the service, rather than granting a
  port the service will not use.
- With no allocation requested, `policy --render` and the generated
  service config are byte-identical to before this change.

## Open Questions

- **The allocate-then-bind race.** Choosing a port means binding `:0`,
  reading the number, and closing — after which another process on the
  host can take it before the sandbox's service binds. The window is
  small but real, and the honest options are retrying on failure,
  holding the socket and passing the fd (the listener-inheritance trick
  `up` already uses for the control socket, but per-service and far more
  intrusive), or accepting and documenting it. Not settled.
- **Whether allocation should avoid the ephemeral range** rather than
  drawing from it, since the kernel hands out the same range to
  unrelated connections and a long-lived service squatting there is
  antisocial. A devcroft-owned range would be more predictable and less
  polite about assuming it is free.
- **What `status` should report when the sandbox is down.** The recorded
  port is still meaningful (it will be reused on the next `up`), but
  reporting it next to a stopped sandbox may read as though something is
  listening.
- **Whether a fixed `ports` entry and an allocated one can coexist**
  in one manifest, and what that means if the fixed one is the thing
  colliding. Leaning: allowed, since some ports genuinely must be fixed,
  with the collision remaining the user's to resolve.

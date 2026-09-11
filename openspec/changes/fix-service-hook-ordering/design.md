## Context

Three facts constrain this, all measured:

1. **Services are started by the keeper at its own startup**, before
   `up` runs the activation script — `up` can only do so once the keeper
   is responsive.
2. **A session started over the control socket dies with its client.**
   `keeper::connection` kills the session's process group after a grace
   period when the client disconnects, asserted by
   `client_disconnect_kills_session_after_grace_period`. So `up` cannot
   start long-lived services itself.
3. **A failing activation script must fail `up` with exit 5**, asserted
   by `a_hook_denied_by_the_policy_fails_up_at_the_keeper_layer`.

Any fix has to satisfy all three at once, which is why the obvious
version of it does not work.

## Goals / Non-Goals

**Goals:**

- The activation script completes before services start.
- Services keep the keeper as their lifetime owner.
- A failing script still fails `up` at layer `keeper`, exit 5.

**Non-Goals:**

- Changing what the activation script is or where it comes from.
- Any change to how services are declared or translated.

## Decisions

### Decision 1: not yet made, and that is the change's substance

Two shapes, and they trade the same pair of properties in opposite
directions:

**A. The keeper runs the script, then starts services.** Ordering becomes
structural — one process, one sequence, no coordination. Lifetime
ownership does not move. What it needs is a path for the result back to
`up`: extending `QueryResult` with the script's outcome is the smallest
version, since `up` already calls `wait_until_responsive` and could poll
once.

Cost: `--skip-hooks` and the failure message move into the keeper, and
`up`'s exit code becomes a function of something it reads rather than
something it did.

**B. `up` runs the script as today, then tells the keeper to start
services.** Failure surfacing does not move at all. What it needs is a
control frame whose effect outlives the connection — every existing frame
is scoped to a session that dies with its client, so this would be the
first that is not, and that exception has to be written carefully or it
becomes a way to leak processes.

Cost: a new protocol frame with different lifetime semantics from every
other one.

**A is the smaller change to reason about; B is the smaller change to the
lifetime contract.** The lifetime contract is the one this project has
repeatedly paid to get right — orphaned services were found twice, once
by process-compose surviving `down` and once by a service ignoring
SIGTERM. That argues for A, and A is what this design leans toward, but
the `QueryResult` extension has not been prototyped and the decision is
recorded as open rather than asserted.

### Decision 2: the ordering guarantee is asserted by observation, not by success

The test must check that the script completed *before* the service
started — by ordering in the keeper's own log — rather than checking that
the service came up. A service can come up for reasons unrelated to the
ordering (state left over from a previous run, which is exactly how this
bug stayed invisible), so a passing "it works" test would not pin the
fix.

## Risks / Trade-offs

- **Reordering touches the teardown guarantee** → the acceptance criteria
  include process absence after `down`, and no change may move service
  registration out of the keeper's registry.
- **A window opens where the sandbox is up, the hook failed, and services
  have not started** → `status` must say something truthful about that
  state; today the question cannot arise because the hook runs after.

## Migration Plan

None. No manifest or provider changes.

## Open Questions

- A or B (decision 1).
- What `status` reports between "hook failed" and "services never
  started", which only exists once services start later than they do now.

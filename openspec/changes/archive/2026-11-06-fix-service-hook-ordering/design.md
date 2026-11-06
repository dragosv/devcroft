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

### Decision 1: A — the keeper runs the hooks, then starts services

**Settled by the project owner.** Both shapes were laid out and traded
the same pair of properties in opposite directions; the argument each
way is kept below, because the losing one is a real cost and not a
strawman.

**A (chosen). The keeper runs the hooks, then starts services.** Ordering
becomes structural — one process, one sequence, no coordination. Lifetime
ownership does not move. What it needs is a path for the result back to
`up`: extending `QueryResult` with the outcome is the smallest version,
since `up` already calls `wait_until_responsive` and could read it once.

Cost, paid knowingly: `--skip-hooks` and the failure message move into
the keeper, and `up`'s exit code becomes a function of something it
*reads* rather than something it *did*. That is a real weakening of a
clean property, and it is why the lifecycle delta states the failure
requirement as surviving "regardless of which process runs the script" —
the guarantee is older than the reordering and does not bend to it.

**B (rejected). `up` runs the hooks as today, then tells the keeper to
start services.** Failure surfacing would not move at all. What it needed
was a control frame whose effect outlives the connection — every existing
frame is scoped to a session that dies with its client, so it would have
been the first exception, and an exception in exactly the mechanism that
prevents leaked processes.

**Why A won.** The lifetime contract is what this project has repeatedly
paid to get right: orphaned services were found twice, once by
process-compose surviving `down` and once by a service ignoring SIGTERM.
B pays in that currency; A pays in error-reporting plumbing, which is
recoverable work rather than a class of bug.

**The measurement that makes A structural rather than merely tidier.**
The two are not ordered today — they *race*, and the hook wins by 9-10 ms
(four consecutive runs; see proposal.md). A race is not fixed by issuing
the spawn calls in a better order, because nothing in that order is a
dependency. A makes service startup *follow the hook's completion* in one
process, which is the only shape that removes the race rather than
re-tuning it. B also removes it, by making the frame the dependency — so
this argument selects A over "reorder the calls", not A over B.

**Group 0 has since measured both open items, and it moved where A's
cost is** — see tasks.md 0.1 and 0.2 for the citations.

The success path turned out **free**: `up` already polls
(`wait_until_responsive`, 50 ms, `up.rs:1333`), so a keeper that runs
hooks, then starts services, then accepts, makes that existing wait the
ordering wait. No protocol change, nothing added.

The failure path turned out **more expensive than estimated above**, and
`QueryResult` is the wrong mechanism for it — not because of polling, but
because answering a `Query` means the keeper is accepting connections,
i.e. the sandbox *came up*. A keeper reporting a failed hook that way
contradicts the lifecycle requirement's other half. What A actually needs
is a failure *channel*: `up` already holds `keeper_pid` and can tell a
dead keeper from a slow one with a `kill(pid, 0)` in the loop it already
runs, but the keeper's reason is written to stderr and `up` reads no
keeper log today. That read is the work A adds.

So the estimate "extending `QueryResult` is the smallest version" is
withdrawn. The decision stands — the reasoning for A was never that it
was free, it was that B pays in the lifetime contract — but 1.3 should be
written against a failure channel rather than against `QueryResult`.

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

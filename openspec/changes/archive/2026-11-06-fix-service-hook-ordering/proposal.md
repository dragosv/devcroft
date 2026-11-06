# Change: fix-service-hook-ordering

## Why

**Services start before the activation hook that prepares what they
need.** Observed in a keeper log, with a devbox mariadb project:

```
services started session=1 pgid=20401
spawn session=2 … bash ".../virtenv/mysql/setup_db.sh"
[hook activation] $ bash ".../virtenv/mysql/setup_db.sh"
```

The hook creates the database's data directory. `mariadbd` is already
running by then, against a directory that does not exist, and fails.

**Re-measured, and the framing above is wrong in a way that matters: the
two are not ordered at all — they race, and the margin is ~9 ms.** With
devcroft's own `hooks.post_create` against a devbox `redis` project, four
consecutive runs:

| run | `post_create` | service `Started` | margin |
|---|---|---|---|
| 1 | 07:24:29.587 | 07:24:29.597 | **+10 ms** |
| 2 | 07:24:35.118 | 07:24:35.127 | +9 ms |
| 3 | 07:24:40.632 | 07:24:40.641 | +9 ms |
| 4 | 07:24:46.189 | 07:24:46.198 | +9 ms |

The keeper log still reads `services started session=1` *before*
`spawn session=2 … post_create`, which is what the observation below was
read from — but that is the **spawn** order, not the order things
actually happen. process-compose has to boot, load its config, start its
scheduler and launch the process; that takes ~9 ms, and a hook spawned
over the control socket in the meantime wins.

**This is worse than a wrong ordering, and it is why this change should
not be closed by making the hook merely earlier.** A trivial hook wins
the race and a real one loses it. `date > file` wins by 9 ms; devbox's
`setup_db.sh`, which sources a generated shell script before doing any
work, does not — which is exactly the observation below. So a test
written with a toy hook **passes**, the feature **fails** in use, and the
difference is a few milliseconds of hook startup. The fix has to
establish a guarantee, not a better average.

The ordering is deliberate and documented — the keeper's own comment says
*"Services start here — at keeper startup, before hooks, which `up` runs
only once this process is responsive"* — and it was correct when written.
No provider's services depended on hook output: flox's `[hook].on-activate`
and devenv's `enterShell` prepare shells, not service state. That stopped
being true when devbox's plugin services arrived, where the plugin's
`setup_db.sh` *is* the service's prerequisite.

**The obvious fix is ruled out, measured.** Move the service spawn from
keeper startup into `up`, after the hook, using the control socket `up`
already uses for hooks. It does not work: a session started over that
socket is killed when the client disconnects, after a grace period —
asserted by `keeper::connection`'s own
`client_disconnect_kills_session_after_grace_period`. `up` exits, so the
services would be reaped seconds later. That is exactly why the keeper
owns them.

So the ordering cannot be fixed by moving *who* starts services without
answering *how the result of the hook reaches `up`*, which is the real
content of this change.

## What Changes

- **The activation script runs before services start.** How is design.md's
  decision; the constraint is that whatever runs it must not tie the
  services' lifetime to `up`'s.
- **A failing activation hook must still fail `up`** at layer `keeper`,
  with exit code 5. That is an existing, tested guarantee
  (`a_hook_denied_by_the_policy_fails_up_at_the_keeper_layer`) and this
  change must not weaken it — which is what makes "just move the hook
  into the keeper" incomplete rather than simple.
- **`--skip-hooks` keeps its promise** that nothing project-supplied
  runs, wherever the hook ends up being run from.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `services`: services SHALL start after the provider's activation
  script has completed, where one exists.
- `lifecycle`: the activation script's failure SHALL continue to fail
  `up` at layer `keeper`, regardless of which process runs it.

## Impact

- **Affected specs**: `services`, `lifecycle`.
- **Affected code**: `src/bin/devcroft.rs`
  (`start_services_if_requested` and the keeper's startup sequence),
  `src/lifecycle/up.rs` (where the activation script is run today),
  possibly `src/keeper/protocol.rs`.
- **This touches the teardown guarantee**, which is why it is proposed
  rather than patched. Several changes exist to establish that no service
  process outlives its sandbox, verified by process absence rather than
  by a stop command's exit status. A reordering that accidentally moves
  service ownership out of the keeper's registry would break that
  silently — the failure mode is an orphaned database, not a test error.

## Success Criteria

- A devbox mariadb project's data directory exists before `mariadbd`
  starts, observed by ordering in the keeper log rather than by the
  service happening to succeed.
- A failing activation hook still fails `up` with exit 5 at layer
  `keeper`.
- `down` still leaves no service process alive, verified by process
  absence.
- flox and devenv projects whose services do not depend on hook output
  behave identically to before.

## Open Questions

- **Where the hook runs.** In the keeper before services, with the result
  reported back to `up`; or in `up` as today, with a new way to tell the
  keeper to start services afterwards. The first keeps lifetime
  ownership where it is and needs a way to surface failure; the second
  keeps failure surfacing where it is and needs a protocol addition that
  does not tie lifetime to the connection.
- **What happens when the hook fails but services are already
  configured.** Today `up` fails and the sandbox is torn down. If
  services start later, there is a window where the sandbox is up, the
  hook has failed, and nothing has started — and `status` should say
  something truthful about it.

- **A hook that needs a *running* service now has no phase.** Ordering
  every hook before every service is right for preparation — `initdb`,
  `setup_db.sh` — and wrong for anything that wants to talk to the
  database it just set up. Dev Containers has a third phase
  (`postAttachCommand`) for exactly this; devcroft has two. Adding a
  third is a scope expansion this change should not make on its own, so
  the consequence is recorded rather than solved: after this change,
  `post_start = "psql -c 'select 1'"` fails where today it works by
  accident, and the accident was a 9 ms race.

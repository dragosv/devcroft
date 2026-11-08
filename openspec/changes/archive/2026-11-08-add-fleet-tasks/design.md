## Context

Fleet starts environments; work arrives over `ssh`/`exec` sessions, whose
lifetime is the client's. Nothing records what an agent is doing.

## Decisions

### D1: A task is a keeper session held by a host-side runner

`fleet run` starts `devcroft __task`, a small host-side process, which opens
an ordinary `Spawn` session on the agent's control socket, appends the
session's output to the task's log, and on `Exit` records the code or
signal. No keeper protocol change. The runner is detached from the command
that started it and runs in the agent's helpers' leaf (created on demand,
with the helpers' limits), so it is budgeted and removed with the agent.

It needs no lifeline: when the agent dies, its keeper's end of the session
closes, which the runner sees.

### D2: Completion is observed, and recorded where the agent cannot write

The task record lives in `<state>/agents/<id>/tasks/<task>.json`. The state
directory is not in any agent's view, so what devcroft reports as the
outcome is what the keeper observed, never what the agent wrote. The log
is beside it.

A record still `running` whose runner is gone is settled by whoever notices
(`stop`, reconcile): `ended with its agent` when the agent is no longer
running, `lost` (with why) when it is. `stop` settles its agent's running
task itself, since it killed it.

### D3: Reports are the agent's, in a file beside its home

`devcroft agent …` inside an agent writes `.devcroft/<id>/report.json`, the
directory holding its home: the same rule `why` uses to find its own
policy (`HOME` is `.devcroft/<x>/home`, which devcroft sets and nothing else
does). No socket, no grant: the workspace is already writable. It survives
the reporting process exiting, and goes with the clone. It is written
atomically (temporary file, rename).

Reports are data from the agent: read with a size cap, shown verbatim but
with control characters escaped, so a message cannot drive the operator's
terminal.

A new task starts with no reports: `fleet run` removes the previous task's,
so an old `attention` or `done` is never shown against new work. The
`DEVCROFT_TASK` variable is given to the task, and recorded with its
reports.

### D4: No severity, no interpretation

Attention is a message and a time (`add-agent-interaction` A2). devcroft
does not classify it, route it, or deliver it anywhere; `fleet ls` showing
it is the contract.

### D5: Progress is reported, or inferred only from output time

devcroft does not judge whether a task progresses. It shows how long the
task has run and how long since its log last grew, and what the agent last
reported with `progress`. Anything more would be devcroft guessing about a
model's reasoning.

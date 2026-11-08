## Why

A fleet agent's lifecycle is solid (a PID 1, a leaf, a lifeline), but
"running" means only that its environment is alive, not that its work is
progressing (adversarial review §4.3). For the unattended product three
things are missing: **which task** an agent is on, **whether it finished**
and how, and **whether it needs a human**. `AgentRecord.attention` was a
boolean nothing ever set: a permanent `false` that the review rightly says
closes nothing.

Work reaches an agent today through `ssh`/`exec`, and fleet never learns
when it ends.

## What Changes

- **Tasks, run by devcroft.** `fleet run <id> -- <cmd>` (and
  `fleet up --agents N -- <cmd>`) runs a command as the agent's task. Each
  task has an ID never reused within its agent (`t1`, `t2`, …), its command,
  start and end, and a log. Completion is **observed**: the exit code or
  signal the keeper reported, or that the task ended with its agent. It is
  recorded host-side, in the fleet's state directory, which no agent can
  see, so an agent cannot misreport it. One active task per agent.
- **Reports, from the agent.** Inside an agent, `devcroft agent attention
  <message>` (and `--clear`), `devcroft agent progress <message>`, and
  `devcroft agent done|failed [message]`. Written beside the agent's home,
  needing no grant it lacks. Shown as **reported**, never merged with what
  was observed: an agent that says `done` and exits 1 shows both.
- `fleet ls` shows each agent's task, its outcome, how long since its last
  output, and attention; `fleet inspect` shows the messages verbatim, with
  control characters escaped, and where the log is.
- The always-false `attention` field is removed from the agent record.
- Each clone's `.devcroft/` is ignored by its own git, so an agent's
  `git add -A` cannot commit devcroft's files (its home, its policy, its
  reports). Found while designing this: nothing prevented it before.

## Capabilities

### New Capabilities

- `agent-tasks`: task identity, observed completion, agent reports, and how
  fleet lists them.

### Modified Capabilities

- None in main specs. Implements the attention half of
  `add-agent-interaction` for fleet agents (its 0.1 and 0.2 are decided
  here); plain sandboxes' `ps`/`status` stay that change's.

## Impact

- `src/fleet/task.rs` (new), `src/fleet/report.rs` (new),
  `src/fleet/supervisor.rs`, `src/fleet/commands.rs`, `src/bin/devcroft.rs`
  (`fleet run`, `agent`, the hidden `__task` runner, `ls`/`inspect`).
- No keeper protocol change: a task is an ordinary keeper session, held by a
  host-side runner.

## 1. Tasks

- [x] 1.1 `fleet::task`: the record, IDs never reused (`last-task`), settle.
- [x] 1.2 The `__task` runner: a keeper session, log, observed outcome.
- [x] 1.3 `fleet run <id> -- <cmd>`; `fleet up … -- <cmd>`; one active task.
- [x] 1.4 `stop` and reconcile settle a running task.

## 2. Reports

- [x] 2.1 `fleet::report`: the file, atomic write, capped read, escaping.
- [x] 2.2 `devcroft agent attention|progress|done|failed`, found by `HOME`.
- [x] 2.3 A new task removes the previous task's reports.

## 3. Listing

- [x] 3.1 `fleet ls`: task, outcome, since last output, attention.
- [x] 3.2 `fleet inspect`: task, log path, reports verbatim (escaped).
- [x] 3.3 Remove `AgentRecord.attention`.

## 4. Git

- [x] 4.1 `.devcroft/.gitignore` in each clone's workspace.

## 5. Tests

- [x] 5.1 Two tasks in turn: IDs, codes, logs.
- [x] 5.2 A second task while one runs is refused.
- [x] 5.3 Stopping the agent mid-task settles the task.
- [x] 5.4 Attention survives the process; a new task starts clean.
- [x] 5.5 Reported versus observed: `done` and exit 1.
- [x] 5.6 Escape sequences are escaped.
- [x] 5.7 `git add -A` stages nothing under `.devcroft/`.

## Notes from building it

- **A runner's EOF is the agent ending, recorded as such.** The first
  version recorded a session that closed without an exit status as `lost`,
  and `stop` lost the race to settle it: killing the agent closed the
  keeper's end before `stop` reached the task. A keeper session ends without
  `Exit` only when the keeper is gone, and an agent's keeper is its main
  process, so it is `ended with its agent`.
- **The clean-slate check was vacuous at first.** A task that raised and
  cleared attention ran before the assertion after `run`, so it passed with
  `report::clear` removed. A quiet task now sits between: any attention seen
  while it runs can only be carried over. The mutant fails it.
- Mutants, each failing a test: no `.gitignore` in the clone (`git status`
  lists `.devcroft/a1/policy.json`); no clearing of reports; no
  one-task-at-a-time check.

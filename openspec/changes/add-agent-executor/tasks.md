## 0. Spike gate — no production code until these pass

Not scheduled: starts after `add-agent-packages` is qualified on the `up` path. Fixture first, then one real harness. Record results in `design.md` under "Evidence" with versions and targets; Linux first, macOS second.

- [ ] 0.1 Hermetic fixture on Linux: a Node 16 project, a Node 24 fixture agent, the parent, two restricted trees and the executor socket created before restriction. Pass review rows R1 (private `fork()` stays on the agent runtime), R2 (direct argv), R3 (descendants and shebangs), R4 (environment and loader), R5 (absolute paths), R6 (git alias and hook).
- [ ] 0.2 Workspace on the same fixture: W1 (edits visible both ways, same physical instance), W2 (cwd, root and two checkouts with one name), W3 (symlinks to granted and ungranted paths).
- [ ] 0.3 Stdio and lifecycle: L1 (PTY, piped, large output, exit and signal, cancel, disconnect, orphan cleanup).
- [ ] 0.4 Credential and network: N1, reported as two properties (the key is unreadable; the project cannot use the agent's authenticated route).
- [ ] 0.5 One real harness from `add-agent-packages`: native exec disabled, executor-backed tool supplied, R2/R3 pass through the real tool, and R7 documented (a local bypass stays in the agent tree, outside the guarantee).
- [ ] 0.6 Stop criteria: report to the owner if any of 0.1–0.4 fails on Linux, or if no qualified harness can disable its native exec path (0.5). The latter leaves mode 1 as the only shipped mode, and that is a valid outcome.

## 1. Topology

- [ ] 1.1 Decide between reusing fleet's `Supervisor` and a smaller parent; implement the parent for mode 2 only.
- [ ] 1.2 Assert agent-less and mode 1 sandboxes are byte-identical in topology and profile.

## 2. Executor

- [ ] 2.1 `src/executor/`: RPC (argv, shell flag, relative cwd, stdio/PTY, resize, signals, cancel, declared deltas); no root, mount, grant, policy or snapshot accepted.
- [ ] 2.2 Snapshot frozen at `up` in the order `up` applies today; `env_clear` + snapshot + deltas per spawn.
- [ ] 2.3 Pair binding by workspace root.

## 3. Policy

- [ ] 3.1 Compile and render two profiles; move the credential route to the agent tree only.

## 4. Harness routing

- [ ] 4.1 Per-recipe mode 2 configuration in `src/agent/`; `status` names the mode and, for mode 1, the reason.

## 5. Tests and docs

- [ ] 5.1 `tests/agent_executor_e2e.rs` from group 0's rows; macOS repetition of 0.1–0.3.
- [ ] 5.2 `docs/threat-model.md` (what mode 2 separates and what it does not), `docs/agents.md` mode column, `README.md`.

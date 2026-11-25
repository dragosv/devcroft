## 0. Spike gate — no production code until these pass

Not scheduled: starts after `add-agent-packages` is qualified on the `up` path, and on macOS after `add-macos-unix-socket-scoping` is verified on hardware. Fixture first, then one real harness. Record results in `design.md` under "Evidence" with versions and targets; Linux first, macOS second.

- [ ] 0.1 Hermetic fixture on Linux: a Node 16 project, a Node 24 fixture agent, the parent, two restricted trees and the executor socket created before restriction. Pass review rows R1 (private `fork()` stays on the agent runtime), R2 (direct `argv`), R3 (descendants and shebangs), R4 (environment, `NODE_OPTIONS` and loader isolation), R5 (absolute paths), R6 (git alias and hook).
- [ ] 0.2 Workspace on the same fixture: W1 (edits visible both ways, same physical instance), W2 (`cwd`, root and two checkouts with one name), W3 (symlinks to granted and ungranted paths).
- [ ] 0.3 Stdio, lifecycle and OSC 7501: L1 (PTY, piped, large output, exit and signal, cancel, disconnect, orphan cleanup), plus verify bidirectional **OSC 7501** (`T01`–`T05`) over `devcroft agent run` against the agent tree's control socket and clean OSC-free `stdout` on `agent acp` and piped MCP executor calls.
- [ ] 0.4 Credential, `[env] forward` collision and network isolation across all three auth modes (`api-key`, `login`, `env`): verify `[env] forward` / `[env.vars]` collisions with broker or `auth = "env"` keys fail before provisioning, and verify the project tree finds no real upstream credential (`api-key` value, `auth = "env"` value, or `auth = "login"` token) anywhere and cannot connect to the agent's proxy port or socket. Include reading the other tree's process environment and arguments (`/proc/<pid>/environ` on Linux, `KERN_PROCARGS2` and `ps -E` on macOS) and attaching to its processes; verify that on macOS `auth = "env"` is refused until process-environment visibility is closed while `api-key` and `login` block connecting to `<agent_port>` via Seatbelt `proxy_only` (design E0).
- [ ] 0.5 Executor socket exclusivity and `network.default = "deny"` eligibility (design E2): verify an MCP server several generations below the agent connects by path while another session runs; verify the project tree cannot `connect()` on Linux (mount view) and on live macOS hardware (Seatbelt `network-outbound` + trailing path denials); confirm mode 2 is refused under `network.default = "allow"`, on a Linux host without a mount view or network namespace, and under any Landlock-only fallback.
- [ ] 0.6 Per-recipe homes, overlap and `mode1_recipes` quarantine (design E5, E6): a project grant of `devcroft-agent-homes`, direct or through a symlink, fails at layer `config`; `O_NOFOLLOW` routing merges preserve user settings; switching recipes in mode 2 gives the new recipe an isolated `<sandbox-id>/<recipe>/home`; a sandbox that ran `claude-code` in mode 1 refuses mode 2 both directly and after an intermediate `up --no-agent --recreate`, with nothing touched; with `--quarantine-agent-state` every file arrives in quarantine, on the same and on another filesystem, readable by neither tree; a `.claude` written by a `[tools]` `claude` in a sandbox that never ran `claude-code` in mode 1 survives `up`.
- [ ] 0.7 Teardown (design E1): `SIGKILL` the parent on macOS and on Linux; `down` and the next `up` leave no process of either tree or proxy.
- [ ] 0.8 One real harness from `add-agent-packages` (`opencode` or `claude-code`): native exec disabled, executor-backed tool supplied, R2/R3 pass through the real tool, and R7 documented (a local bypass stays in the agent tree, outside the guarantee). Record whether the editor's `session/new` can re-enable the native tool or remove the MCP server, and which transport carries a PTY, if any. List every workspace file the harness runs or obeys, and show with a planted project-level hook and MCP declaration what reaches the agent tree, with and without any lever that disables them.
- [ ] 0.9 Stop criteria: report to the owner if any of 0.1–0.7 fails on Linux or macOS, or if no qualified harness can disable its native exec path without ACP rewriting (0.8). The latter leaves mode 1 as the only shipped mode, and that is a valid outcome. Also report if 0.8 shows that project files reach credentials in the agent tree on every recipe: the owner then decides whether mode 2's remaining separation is worth building.

## 1. Topology and Mode 1 → Mode 2 transition

- [ ] 1.1 Decide between reusing fleet's `Supervisor` and a smaller parent; `[agent] mode`, `up --agent-mode`, recorded with the selection; eligibility refused before teardown (design E0).
- [ ] 1.2 Assert agent-less and mode 1 sandboxes are byte-identical in topology and profile.
- [ ] 1.3 Pid records for both trees and both proxies; `down` and orphan recovery stop each recorded group.
- [ ] 1.4 Second control socket for the agent tree; command-to-tree mapping, innermost-root discovery and valid `up [--name <n>]` remedies (design E7).
- [ ] 1.5 Per-recipe agent `HOME` under `~/.local/share/devcroft-agent-homes/<sandbox-id>/<recipe>/home`; explicit sandbox-directory variable read first by `own_sandbox_dir`; `Meta.mode1_recipes` refusal (including across `--no-agent`), `--quarantine-agent-state` and the revocation notice (design E6).

## 2. Executor

- [ ] 2.1 `src/executor/`: RPC (`argv`, shell flag, relative `cwd`, stdio/PTY, resize, signals, cancel, declared deltas); no root, mount, grant, policy or snapshot accepted.
- [ ] 2.2 Pre-provisioning collision refusal across `manifest.env.vars`, `manifest.env.forward` and `[credentials] env` for all recipe auth/launch variables; snapshot frozen at `up`; `env_clear` + snapshot + deltas per spawn (design E3).
- [ ] 2.3 Pair binding by canonical workspace root.

## 3. Policy, socket denials and read/write overlap check

- [ ] 3.1 Add `agent_home_overlap` in `src/policy/capability_set.rs` rejecting both `AccessMode::Read` and `AccessMode::ReadWrite` canonical project grants overlapping `devcroft-agent-homes` or the agent-sockets root; add trailing Seatbelt denials for the agent-sockets directory on macOS; compile and render two profiles isolating all three auth modes (`api-key`, `login`, `env`) to the agent tree (design E2, E5).
- [ ] 3.2 Two egress proxies (allowlist, token, socket, pidfile, log each) and, on Linux, two namespaces and relays.

## 4. Harness routing

- [ ] 4.1 Per-recipe mode 2 configuration merged via `O_NOFOLLOW` (`OwnedDir`) into the agent `HOME` and included in `recipe_digest`; `status` names the mode; a recipe version not qualified is refused in mode 2.

## 5. Tests and docs

- [ ] 5.1 `tests/agent_executor_e2e.rs` covering Group 0's rows, `[env] forward` collision refusal, grant overlap refusal on `devcroft-agent-homes`, each eligibility refusal (including `auth = "env"` refused on macOS while `api-key`/`login` work when Seatbelt denials hold, and recipe switch to an ineligible recipe failing closed in mode 2), mode unchanged by upgrade and by editing `[agent]`, per-recipe `HOME` isolation on mode 2 recipe switch, `mode1_recipes` quarantine refused both directly and after `up --no-agent --recreate` and then quarantined with no file lost, unrelated `state_paths` untouched, `O_NOFOLLOW` symlink rejection in agent `HOME`, and OSC 7501 over the agent tree's PTY on both Linux and macOS.
- [ ] 5.2 `docs/threat-model.md` (what mode 2 separates, revoking a mode 1 login, and what each tree can still do to the other), `docs/agents.md` mode column, `README.md`.

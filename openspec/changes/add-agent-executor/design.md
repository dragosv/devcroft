## Question

Can a supported harness send its real test command to a separate project process tree while the project has no access to the agent’s credential? Start after one recipe passes its terminal qualification. Use a fresh disposable sandbox, a synthetic credential and the project’s actual toolchain. The experiment runs on Linux only. macOS has no process-visibility separation between sandboxes, so it waits for its own measurements.

## Experiment

Start with a Linux fixture (`network.default = "deny"`, private mount view) and two sibling trees: separate homes and network policies, a shared workspace and a pair-specific executor. Create listeners before applying restrictions.

**Topology.** Keep the project keeper, with its SSH and services, and give it a narrow typed ingress for the executor. Do not publish the keeper's existing wire protocol (`cmd`, `cwd`, `env`) as the executor. Put a separate agent keeper beside it, starting one harness per agent session. A minimal per-sandbox supervisor holds both keepers. Reuse fleet's primitives for descriptor handoff, PID 1 and the lifeline, not the whole `fleet::Supervisor`. Specify which process creates, inherits and closes each descriptor.

The executor ingress has these properties:

- the pair and the generation are fixed by the server, not by the caller;
- `cwd` is resolved inside the project's view;
- only explicitly allowed environment deltas pass, and none may change the executable's source, the loader, `HOME` or the proxy;
- each session owns its processes;
- output, time and concurrency are limited;
- cancel and disconnect have defined semantics.

The pair moves through `Preparing → Ready → Stopping | Failed` under one generation. `Ready` requires answers from the project keeper, the agent keeper and the executor for that generation, together with every mandatory route. A listener that only accepts in its backlog is not an answer.

Build the project environment snapshot with one pure function, in this order:

1. the provider environment;
2. the tooling layer;
3. the manifest’s and project’s credentials;
4. the project proxy variables;
5. the activation set and unset;
6. child redirect rules;
7. remove keeper-internal values;
8. remove every agent-only value;
9. freeze.

Refuse any `[env] forward` or `[env.vars]` collision on the agent’s credential variables before that snapshot. Verify canonicalized project/provider/tooling filesystem grants for both `Read` and `ReadWrite`, so that no project read grant covers the isolated agent `HOME` or the executor socket directory. (`artifact_store_write_overlap` checks `ReadWrite` only.)

Compare the agent’s own runtime with project commands started by direct arguments and by the shell, including descendants. Record working directory, runtime version, exit status, cancellation and cleanup.

Bind the pair by `(canonical project root, sandbox name)` plus a generation nonce for the live connection, never by root alone, since one root can hold several named sandboxes and successive generations. Resolve the command’s working directory by descriptor and `fchdir`, not by canonicalizing a path and then calling `chdir` on it.

Repeat the edit-and-test path with one real harness. Identify the supported setting that routes project commands to the executor and whether workspace or session settings can change it. Candidate routes:

- ACP client-side terminal capabilities;
- Codex App Server’s client-declared tools with its own shell tools disabled;
- an MCP tool, whose component, argv mapping, working directory, environment, framing, cancellation and output limits must then be specified, together with protection against a project-level MCP entry replacing it.

Qualify each `(component, platform, launch path (ACP or CLI), routing mechanism)` separately. The ACP adapter and the interactive build may be different binaries. If only the ACP path passes, only ACP is offered in Mode 2. If no pinned harness can disable its native exec without devcroft rewriting ACP, Mode 1 stays the only shipped mode. Record any local commands outside the chosen route. Confirm that project configuration (`[env] forward`, `[env.vars]`, `filesystem.read` / `read_write`) cannot put the synthetic credential or the agent’s `HOME` or socket into the project’s environment or grants. Confirm that the project cannot reach the agent’s proxy or credential route. The shared workspace still allows project files to influence the agent.

## Acceptance cases

The experiment runs every case below on the Linux fixture, and again with the real harness where the case applies. A case that did not run is reported as not run, never as passed. These are the cases of the 2026-11-19 runtime/workspace separation review, kept here so that the change does not depend on untracked notes. Package-level cases (provisioning offline, store versus state, atomic update, GC) belong to `add-agent-packages`.

| ID | Probe | Required result |
|---|---|---|
| `R1` | The harness reports its runtime version and forks a worker | Harness and worker run on the agent's private runtime (Node 24), and the worker is not rerouted as a project command |
| `R2` | The harness's tool asks for a direct-argv exec (`node -p process.versions.node`, then `npm test`) | Supported operations reach the executor and report the project's runtime (Node 16); an adapter that executes locally fails the case |
| `R3` | `npm test` starts a `#!/usr/bin/env node` script that starts `node` and a grandchild | Every generation runs on the project's runtime with the project's env, cwd and `node_modules/.bin`, none taken from the agent |
| `R4` | The agent's environment carries other `PATH`, `NODE_OPTIONS`, `NODE_PATH`, `LD_PRELOAD`, `LD_LIBRARY_PATH`, npm variables, `GIT_DIR`, `GIT_WORK_TREE` and a private marker, and the request tries to override `PATH` | The executor receives no agent-only value, the project snapshot is intact, and runtime, loader and root overrides are refused |
| `R5` | The request names the project runtime's absolute path, then the agent's private runtime's absolute path | The first runs under the project's policy; the second is unavailable or refused, never silently rewritten |
| `R6` | `git` runs an `!node …` alias and a `pre-commit` hook | Both run on the project's runtime, in the right root, without the agent's `GIT_DIR`/`GIT_WORK_TREE` |
| `R7` | A fixture bypasses the route and runs `process.execPath workspace/project.js` locally | It is classified as a local agent child, not a project command; if the harness's real shell or test path does this, the harness does not qualify |
| `W1` | The agent writes a marker, the executor reads it and writes another, and the agent reads that | Changes are visible at once on the same physical workspace, not on two clones |
| `W2` | Relative, absolute, `..` and differently spelled `cwd`, and two checkouts with the same sandbox name | `cwd` is interpreted inside the pair's root, and no marker or state crosses into the other checkout |
| `W3` | Workspace symlinks to a granted directory, an ungranted one, and the agent's install or state | Access follows each tree's policy, measured on the backend rather than validated lexically |
| `W4` | Several pairs share one read-only package and build and commit in parallel | Each pair keeps its own executor, state and credential route; nothing is shared implicitly |
| `L1` | PTY and piped I/O, large output, non-zero exit, `SIGINT` and cancel, client disconnect, timeout, a child that outlives the agent | Ordering, backpressure, exit and signal are correct; after crash or `down` no child or broker with unrecorded authority remains |
| `S2` | The project's lockfile and the agent's recipe change after `up`, then the executor restarts (if recovery is in scope) and the pair is recreated | Snapshots hold until recreation; recovery never resolves the provider from the agent's environment or persists the real secret |
| `N1` | The project searches for the secret in env, files, logs and state and tries the agent's proxy, token and socket; the agent tries an unlisted host | The real key is unreadable **and** the agent's authenticated route is unusable by the project; both are reported separately |

## Decision and mandatory gates for any follow-up Mode 2 proposal

Record the tested recipe/version, platform, authentication method and results. If routing or credential separation fails, retain the shared-sandbox offering and document the limit.

A positive result qualifies a follow-up production Mode 2 proposal only if that proposal enforces all nine gates below. None of them may degrade.

1. **Mandatory `network.default = "deny"` and private view.** Mode 2 is refused, never downgraded, when `network.default = "allow"`, or when Linux runs in Landlock-only fallback without a mount view or network namespace. Eligibility is decided on the final topology, after every fallback, not on the manifest alone.
2. **Cross-tree process isolation in every auth mode.** For `api-key`, `login` and `env` alike, deny the other tree environment and argument inspection (`/proc/<pid>/environ`, `KERN_PROCARGS2`), attachment (`ptrace`, `task_for_pid`) and signalling. The first Mode 2 is Linux-only. macOS stays ineligible until every one of these denials is measured.
3. **Pre-snapshot credential collision refusal.** Refuse `[env] forward` and `[env.vars]` entries naming any credential variable of the selected recipe, in every mode, before the snapshot above is captured (`src/lifecycle/up.rs`). That way `env_clear` plus snapshot restore cannot re-expose the secret.
4. **Canonical `Read` + `ReadWrite` grant overlap refusal, across live sandboxes.** Check canonicalized grants against the isolated agent `HOME` root and the executor socket directory, refusing `Read` as well as `ReadWrite` overlaps. Check them in the new sandbox’s profiles and in every live sandbox’s compiled profile, naming any live sandbox that must be recreated first.
5. **Host-side helpers start from a cleared environment.** The parent, both proxies, relays and any MCP bridge start from `env_clear` plus an explicit list. A broker’s key reaches only the agent’s dedicated proxy, through a pipe rather than the environment. The credential exists only in the agent tree and its broker, never in the project tree, a project helper, the snapshot, logs or metadata.
6. **Crash-atomic startup and fate-sharing.** A startup gate holds each child before exec and restriction until the parent has durably recorded its identity (atomic rename, directory fsync). Helpers hold a lifeline, as `src/fleet/lifeline.rs` does. The death of an essential component marks the sandbox stale and ends its siblings. Restart is optional; stale marking and cleanup are part of the first slice.
7. **No workspace-reachable control socket.** Each tree gets its own mount plan. A socket that one tree must not reach, such as the services supervisor’s `.devcroft/<name>/run/svc.sock`, the executor socket or the agent’s proxy socket, is absent from that tree’s view.
8. **Fresh sandbox only.** Mode 2 is offered only on a sandbox whose `Meta.mode1_recipes` is empty and whose `HOME` has never held agent state. Migrating a Mode 1 sandbox is a later change. That change works over the union of every recorded recipe’s `state_paths`, with an idempotent journal and a stable transaction ID, and with failure injection after each copy, fsync, rename and removal.
9. **A started harness is not an available executor.** Each session checks that the qualified routing tool is actually offered to the model, and that native exec is actually absent. If either check fails, Mode 2 refuses that session. It never continues with local exec.

Also part of any follow-up:

- Teardown at `down` is guaranteed on Linux by the PID namespace. Per-session cleanup promises only the session’s process group, or uses a cgroup kill. Fixtures cover `setsid`, `setpgid`, double fork and inherited stdio.
- `policy --render` and `why` report each tree separately (`--tree project|agent`) from two profile files, and never give one combined verdict.

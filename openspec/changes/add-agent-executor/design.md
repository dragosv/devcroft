## Question

Can a supported harness send its real test command to a separate project process tree while the project has no access to the agent’s credential? Start after one recipe passes its terminal qualification. Use a fresh disposable sandbox, a synthetic credential and the project’s actual toolchain. The experiment runs on Linux only. macOS has no process-visibility separation between sandboxes, so it waits for its own measurements.

## Experiment

Start with a Linux fixture (`network.default = "deny"`, private mount view) and two sibling trees: separate homes and network policies, a shared workspace and a pair-specific executor. Create listeners before applying restrictions.

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

Record any local commands outside the chosen route. Confirm that project configuration (`[env] forward`, `[env.vars]`, `filesystem.read` / `read_write`) cannot put the synthetic credential or the agent’s `HOME` or socket into the project’s environment or grants. Confirm that the project cannot reach the agent’s proxy or credential route. The shared workspace still allows project files to influence the agent.

## Decision and mandatory gates for any follow-up Mode 2 proposal

Record the tested recipe/version, platform, authentication method and results. If routing or credential separation fails, retain the shared-sandbox offering and document the limit.

A positive result qualifies a follow-up production Mode 2 proposal only if that proposal enforces every gate below. None of them may degrade.

1. **Mandatory `network.default = "deny"` and private view.** Mode 2 is refused, never downgraded, when `network.default = "allow"`, or when Linux runs in Landlock-only fallback without a mount view or network namespace. Eligibility is decided on the final topology, after every fallback, not on the manifest alone.
2. **Cross-tree process isolation in every auth mode.** For `api-key`, `login` and `env` alike, deny the other tree environment and argument inspection (`/proc/<pid>/environ`, `KERN_PROCARGS2`), attachment (`ptrace`, `task_for_pid`) and signalling. The first Mode 2 is Linux-only. macOS stays ineligible until every one of these denials is measured.
3. **Pre-snapshot credential collision refusal.** Refuse `[env] forward` and `[env.vars]` entries naming any credential variable of the selected recipe, in every mode, before the snapshot above is captured (`src/lifecycle/up.rs`). That way `env_clear` plus snapshot restore cannot re-expose the secret.
4. **Canonical `Read` + `ReadWrite` grant overlap refusal, across live sandboxes.** Check canonicalized grants against the isolated agent `HOME` root and the executor socket directory, refusing `Read` as well as `ReadWrite` overlaps. Check them in the new sandbox’s profiles and in every live sandbox’s compiled profile, naming any live sandbox that must be recreated first.
5. **Host-side helpers start from a cleared environment.** The parent, both proxies, relays and any MCP bridge start from `env_clear` plus an explicit list. A broker’s key reaches only the agent’s dedicated proxy, through a pipe rather than the environment. The credential exists only in the agent tree and its broker, never in the project tree, a project helper, the snapshot, logs or metadata.
6. **Crash-atomic startup and fate-sharing.** A startup gate holds each child before exec and restriction until the parent has durably recorded its identity (atomic rename, directory fsync). Helpers hold a lifeline, as `src/fleet/lifeline.rs` does. The death of an essential component marks the sandbox stale and ends its siblings. Restart is optional; stale marking and cleanup are part of the first slice.
7. **No workspace-reachable control socket.** Each tree gets its own mount plan. A socket that one tree must not reach, such as the services supervisor’s `.devcroft/<name>/run/svc.sock`, the executor socket or the agent’s proxy socket, is absent from that tree’s view.
8. **Fresh sandbox only.** Mode 2 is offered only on a sandbox whose `Meta.mode1_recipes` is empty and whose `HOME` has never held agent state. Migrating a Mode 1 sandbox is a later change. That change works over the union of every recorded recipe’s `state_paths`, with an idempotent journal and a stable transaction ID, and with failure injection after each copy, fsync, rename and removal.

Also part of any follow-up:

- Teardown at `down` is guaranteed on Linux by the PID namespace. Per-session cleanup promises only the session’s process group, or uses a cgroup kill. Fixtures cover `setsid`, `setpgid`, double fork and inherited stdio.
- `policy --render` and `why` report each tree separately (`--tree project|agent`) from two profile files, and never give one combined verdict.

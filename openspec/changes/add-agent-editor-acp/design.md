## Context

Use the first packaged agent’s native ACP endpoint. The editor starts a host process and expects a clean JSON-RPC stream. A project’s provider can take minutes to prepare, longer than any editor waits for `initialize`.

## Decisions

**Attach only.** `agent acp` connects to a sandbox that is already up, and never starts, recreates or downloads anything. The user brings the sandbox up from a terminal with `devcroft agent run <recipe>` or `devcroft up --agent <recipe>`. When the sandbox is down, or its lifecycle lock is held by an `up` in progress, `agent acp` fails at once on `stderr` with the exact command (`up`, or `status` while preparation runs) and starts nothing. There is no deadline, no background `up`, no failure marker and no retry protocol. Those belong to a later auto-start change, which must define preparation per generation (`Idle → Preparing{generation} → Ready | Failed`), not as one global marker.

A sandbox counts as up only when its keeper answers for the generation recorded in `Meta`. An accepted connection on its socket does not count, because the listener exists before the keeper restricts itself and admits sessions.

Share target discovery, recorded selection, `recipe_digest` staleness, keeper-version checks and `cd <quoted project_root> && up [--name <n>]` remediation commands with `agent run`. A running sandbox uses its recorded recipe even when its `recipe_digest` is stale, with one `stderr` line naming `up --recreate --agent-upgrade`. The object of a running sandbox is referenced by its `Meta`, so a sweep cannot remove it while the sandbox exists.

For GUI launch, print an absolute executable path and one measured editor configuration. Because `agent acp` never runs `up`, the editor needs no credential in its environment. The key reached the broker when the user ran `up`. The user may still configure a bounded credential command, so that `up` itself needs no exported key. It is a non-empty argument vector that only `up` executes, once, when the host key variable is unset or empty (`""`). It fails at layer `config` on non-zero exit, timeout or empty output (after trimming trailing newlines). Preflight and diagnostics only inspect its configuration. Keep its output out of logs, policy and metadata.

Spawn with `pty: None` and apply the recipe’s launch restrictions. Relay requests and replies as bytes. Send diagnostics to `stderr`. Forward EOF, allow a grace period and then end the session’s process group. A descendant that left the group (`setsid`, double fork) is not covered and lives until `down`. The qualification records whether the qualified harness does that. Request cancellation travels to the agent as an ordinary protocol message.

## Evidence

The evidence comes in three layers, and each proves something the others cannot:

1. **ACP protocol.** Testy (`agent-client-protocol-test`) at a pinned revision is the positive ACP peer for protocol cases: initialize, sessions, prompt, cancel and disconnect. It is a git dev-dependency, because the crate is not published. Verify that `cargo package` drops it.
2. **Effects in the OS.** Testy is not an oracle for authentication or for “denied means no effect”. Those assertions come from `devcroft-test-agent` and OS probes that observe the effect directly.
3. **The real harness.** Keep one sanitized ACP transcript from the qualified recipe, to catch where the real harness differs from the generic peer.

An `initialize` proves the transport, never authentication or model access.

Start one editor from the OS GUI with minimal `PATH` and no exported key, against a sandbox brought up from a terminal. Candidate clients take a literal argument vector with no shell, such as T3 Code’s local ACP command or Zed’s agent servers. Verify:

- warm `initialize` time against the editor’s timeout;
- immediate actionable refusal on a down, preparing, nested or ambiguous checkout;
- an edit followed by a project test;
- two worktrees with a session each;
- a policy denial that holds when the client approves the call or runs the harness without its own prompts. Record editor version, working directory, initialize time and disconnect behavior. `T11` from the [terminal transport cases](../support-osc7501-terminal/design.md) belongs here: the ACP `stdout` carries no devcroft output and no OSC sequence.

## Context

Use the first packaged agent's native ACP endpoint. The editor starts a host process and expects a clean JSON-RPC stream. A project’s provider can take longer to prepare than the editor waits for initialization.

## Decisions

Share target discovery, recorded selection, `recipe_digest` staleness, keeper-version checks and `cd <quoted project_root> && up [--name <n>]` remediation commands with `agent run`. For GUI launch, print an absolute executable path and one measured editor configuration. The user may configure a bounded credential command as a non-empty argument vector. Only `up` executes it, once when the host key variable is unset or empty (`""`), failing at layer `config` on non-zero exit, timeout or empty output (after trimming trailing newlines); preflight and diagnostics only inspect its configuration. Keep its output out of logs, policy and metadata.

Check the local object, project lockfiles, target recipe selection (failing immediately if no `Meta` exists and neither `<recipe>` nor `[agent]` selects one) and auth prerequisites first. If a brand-new or matching stopped sandbox can start, detach one `up` (passing `--agent <recipe>` when no `Meta` exists and `<recipe>` was passed on the command line) with its own process session, closed stdin and both outputs in a preparation log. Bound the wait by the qualified editor’s initialization timeout (measure a 10-second starting value). Serialize preparations with the lifecycle lock. Preserve ongoing work when the editor exits, report a previous failure and retry when it next connects. A running matching sandbox uses its recorded recipe even if its `recipe_digest` is stale.

Spawn with `pty: None` and apply the recipe’s launch restrictions. Relay requests and replies as bytes. Send diagnostics to stderr. Forward EOF, allow a grace period and then end the session’s process group. Request cancellation travels to the agent as an ordinary protocol message.

## Evidence

Start one editor from the OS GUI with minimal PATH and no exported key. Verify cold and warm startup, actionable failure from a nested or ambiguous checkout, and an edit followed by a project test. Record editor version, working directory, deadline, retry behavior and disconnect behavior.

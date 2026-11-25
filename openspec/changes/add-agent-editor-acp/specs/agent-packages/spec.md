## ADDED Requirements

### Requirement: ACP endpoint discovery, bounded startup and valid remedies
`devcroft agent acp [<recipe>]` SHALL run on the host and resolve its target sandbox by
`--sandbox <name>`, or else by comparing both the nearest upward `devcroft.toml`
root and the innermost recorded `Meta` `project_root` containing canonical
`cwd` and choosing the deeper (more specific) root. Ambiguous sandboxes sharing
the same `project_root` SHALL fail at layer `config` naming each candidate and
`--sandbox`. `agent acp` SHALL refuse `acp = "none"` naming `devcroft agent run
<recipe>`, SHALL fail immediately on `stderr` at layer `config` without spawning
`up` when no `Meta` exists and neither `<recipe>` nor `[agent]` selects a
recipe, SHALL fail immediately on `stderr` naming `[cd <quoted project_root> &&
]devcroft up [--name <n>] --recreate [--agent <recipe>]` whenever `Meta`
already exists on disk (running or stopped) with `Unset`, `Disabled` or a
recipe other than the target one, and SHALL fail immediately on `stderr`
without downloading or executing any credential command when the agent object
or a preflight prerequisite is missing. When a brand-new sandbox (no `Meta`) or
a stopped sandbox already recording the target recipe is down, `agent acp`
SHALL start `up` as a separate process that owns its own completion (passing
`--agent <recipe>` when no `Meta` exists and `<recipe>` was given on the
command line) and wait for it under a bounded deadline. At the deadline, or
when the client ends, that `up` SHALL be left to finish; `agent acp` SHALL exit
non-zero on `stderr` naming the log and the exact preparation command. While a
lifecycle operation is in progress it SHALL wait under the same deadline and
SHALL NOT start another. If an earlier background `up` failed, the next `agent
acp` SHALL report that earlier error on `stderr`, clear the failure marker and
start a fresh `up` under the deadline. A running sandbox whose `recipe_digest`
is stale SHALL be used with one `stderr` line naming the recreate command. It
SHALL pass `stdin` and `stdout` through unmodified (`pty: None`) with zero
devcroft messages or OSC sequences on `stdout`, including while `up` runs.

#### Scenario: Editor handshake on a first-time checkout without global agent config
- **WHEN** `~/.config/devcroft/config.toml` has no `[agent]`, the `<recipe>` object is installed, the project environment activates within the startup deadline, and a GUI-launched ACP client runs `devcroft agent acp <recipe>` in a project where `up` has never run
- **THEN** `agent acp` brings the sandbox up with `--agent <recipe>` and the user configuration and returns the agent's `initialize` response with no devcroft output on `stdout`

#### Scenario: Nested subproject does not attach to parent sandbox
- **WHEN** parent `/repo` is already up with recorded `Meta`, and an editor runs `devcroft agent acp` inside `/repo/sub` which has its own `devcroft.toml` and no recorded `Meta` yet
- **THEN** `agent acp` targets `/repo/sub`'s manifest rather than attaching to `/repo`'s running sandbox

#### Scenario: Cold provisioning outlives the deadline
- **WHEN** `agent acp` auto-starts a down sandbox whose provisioning exceeds the deadline
- **THEN** it exits non-zero on `stderr` naming the log and `devcroft up` for that project root, and the `up` continues to completion

#### Scenario: Editor retries during provisioning
- **WHEN** the editor runs `agent acp` again while that `up` is in progress
- **THEN** no second `up` starts, and a retry after it completes returns the `initialize` response

#### Scenario: Background up failed and next retry recovers
- **WHEN** a background `up` failed, the user fixes the cause, and `agent acp` runs again
- **THEN** `stderr` reports the earlier failure summary, clears the marker, starts a fresh `up`, and returns the `initialize` response once ready

#### Scenario: Stopped sandbox that never had a selection is not widened by agent acp
- **WHEN** a stopped sandbox on disk has `Meta` with no recorded agent selection (`Unset`) and `agent acp` (with or without `<recipe>`) runs
- **THEN** `agent acp` fails immediately without starting `up`, naming `devcroft up --recreate` (plus `--agent <recipe>` when one was passed)

#### Scenario: Different recipe on a named sandbox prints a valid command
- **WHEN** a sandbox brought up at `/repo` with `devcroft up --name wt2` runs another recipe, or none
- **THEN** `agent acp <recipe> --sandbox wt2` fails naming `devcroft up --name wt2 --recreate --agent <recipe>` (rooted at `/repo`)

#### Scenario: Upgrade does not break a running editor setup
- **WHEN** devcroft is upgraded to a release whose recipe differs and `agent acp` runs on a sandbox still up with the old one
- **THEN** it returns the `initialize` response and prints one staleness line on `stderr`

#### Scenario: Object not installed
- **WHEN** `agent acp` runs and the recipe's object is absent
- **THEN** it fails at once on `stderr`, naming `devcroft agent install <recipe>`

#### Scenario: Editor started outside any project
- **WHEN** `agent acp` runs with `cwd = /` and no `--sandbox`
- **THEN** it fails saying no sandbox or `devcroft.toml` contains the directory and names `--sandbox`

### Requirement: ACP session lifecycle
When the ACP client closes `stdin`, devcroft SHALL forward it and give the agent
a grace period to exit. After that period, or when the client process dies, the
session's process group SHALL be ended. The sandbox, its services and every
other session SHALL stay up. devcroft SHALL NOT interpret ACP cancellation.

#### Scenario: Editor closes
- **WHEN** the editor exits while the agent is running a command
- **THEN** after the grace period no process of that session remains, and the sandbox and its services are still up

#### Scenario: Two editors
- **WHEN** two clients run `agent acp` on one sandbox and one disconnects
- **THEN** the other session continues

#### Scenario: Request cancelled
- **WHEN** the client cancels a request without closing the session
- **THEN** the message reaches the agent unmodified and the session continues

### Requirement: Native editor session uses the packaged runtime

The endpoint SHALL apply the same launch environment restrictions as `agent run`, confirm keeper support before connecting, and qualify a file edit and project test in one GUI-launched editor. A CLI-only recipe SHALL fail with a remedy naming `agent run`.

#### Scenario: First task from the editor
- **WHEN** a user launches the qualified editor from the OS GUI, edits a file through the agent and asks it to run the project's test
- **THEN** the intended checkout contains the edit and the test runs with the project's tools

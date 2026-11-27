## ADDED Requirements

### Requirement: ACP attaches to a running sandbox and starts nothing
`devcroft agent acp [<recipe>]` SHALL run on the host and resolve its target sandbox by
`--sandbox <name>`, or else by comparing both the nearest upward `devcroft.toml`
root and the innermost recorded `Meta` `project_root` containing canonical
`cwd` and choosing the deeper (more specific) root. Ambiguous sandboxes sharing
the same `project_root` SHALL fail at layer `config` naming each candidate and
`--sandbox`. `agent acp` SHALL NOT start, recreate or download anything, and
SHALL NOT execute a credential command. It SHALL treat a sandbox as running
only when its keeper answers for the generation recorded in `Meta`, never on an
accepted socket connection alone. Every refusal SHALL be immediate, on
`stderr`, with an exact command formatted as `[cd <quoted project_root> &&
]devcroft …` and `--name <n>` whenever `sandbox_name != manifest.sandbox.name`.
The refusals are:

- `acp = "none"`, naming `devcroft agent run <recipe>`;
- no `Meta` and neither `<recipe>` nor `[agent]` selecting a recipe, at layer `config`;
- `Meta` with `Unset`, `Disabled` or a recipe other than the target one, naming `devcroft up [--name <n>] --recreate [--agent <recipe>]`;
- a sandbox that is down, or has no `Meta` yet, naming `devcroft up [--name <n>] [--agent <recipe>]` (or `devcroft agent run <recipe>`);
- a sandbox whose lifecycle operation is in progress, saying so and naming `devcroft status [<name>]`.

A running sandbox whose `recipe_digest` is stale SHALL be used, with one
`stderr` line naming the recreate command. `agent acp` SHALL pass `stdin` and
`stdout` through unmodified (`pty: None`), with zero devcroft messages or OSC
sequences on `stdout`.

#### Scenario: Editor attaches to a running sandbox
- **WHEN** the user ran `devcroft agent run <recipe>` in a terminal, and a GUI-launched ACP client with no key in its environment then runs `devcroft agent acp <recipe>` in that project
- **THEN** the client receives the agent's `initialize` response, with no devcroft output on `stdout`

#### Scenario: Sandbox down
- **WHEN** `agent acp` runs on a project whose sandbox is stopped or was never brought up
- **THEN** it exits non-zero at once naming the `up` command for that project root, and no `up`, download or credential command runs

#### Scenario: Preparation in progress
- **WHEN** `agent acp` runs while an `up` of that sandbox holds its lifecycle lock
- **THEN** it exits non-zero at once saying preparation is in progress and naming `devcroft status`, and starts no second `up`

#### Scenario: Listener present before the keeper is ready
- **WHEN** the sandbox's socket accepts connections but its keeper has not yet answered for the recorded generation
- **THEN** `agent acp` treats it as preparing, not as running

#### Scenario: Nested subproject does not attach to parent sandbox
- **WHEN** parent `/repo` is up with recorded `Meta`, and an editor runs `devcroft agent acp <recipe>` inside `/repo/sub`, which has its own `devcroft.toml` and no recorded `Meta` yet
- **THEN** `agent acp` targets `/repo/sub` rather than `/repo`'s sandbox, and fails naming `cd /repo/sub && devcroft up --agent <recipe>`

#### Scenario: Stopped sandbox that never had a selection is not widened by agent acp
- **WHEN** a stopped sandbox on disk has `Meta` with no recorded agent selection (`Unset`) and `agent acp` (with or without `<recipe>`) runs
- **THEN** `agent acp` fails immediately without starting `up`, naming `devcroft up --recreate` (plus `--agent <recipe>` when one was passed)

#### Scenario: Different recipe on a named sandbox prints a valid command
- **WHEN** a sandbox brought up at `/repo` with `devcroft up --name wt2` runs another recipe, or none
- **THEN** `agent acp <recipe> --sandbox wt2` fails naming `devcroft up --name wt2 --recreate --agent <recipe>` (rooted at `/repo`)

#### Scenario: Upgrade does not break a running editor setup
- **WHEN** devcroft is upgraded to a release whose recipe differs and `agent acp` runs on a sandbox still up with the old one
- **THEN** it returns the `initialize` response and prints one staleness line on `stderr`

#### Scenario: Editor started outside any project
- **WHEN** `agent acp` runs with `cwd = /` and no `--sandbox`
- **THEN** it fails saying no sandbox or `devcroft.toml` contains the directory and names `--sandbox`

### Requirement: ACP session lifecycle
When the ACP client closes `stdin`, devcroft SHALL forward it and give the agent
a grace period to exit. After that period, or when the client process dies, the
session's process group SHALL be ended. A descendant that left that process
group is outside this guarantee and SHALL be ended by `down`; the published
qualification SHALL say whether the qualified harness leaves any. The sandbox,
its services and every other session SHALL stay up. devcroft SHALL NOT
interpret ACP cancellation.

#### Scenario: Editor closes
- **WHEN** the editor exits while the agent is running a command
- **THEN** after the grace period no process remains in that session's process group, and the sandbox and its services are still up

#### Scenario: Descendant that left the group
- **WHEN** a fixture agent starts a child with `setsid` and the editor exits
- **THEN** the child is not reported as cleaned up, and `down` ends it

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

## ADDED Requirements

### Requirement: Agent commands, editor onboarding and visibility
The CLI SHALL provide the host-side verbs `devcroft agent list` (each recipe
with its versions, ACP mode, supported auth modes, platforms, whether it is
installed and whether it is pinned), `agent install <recipe> [--version <v>]`,
`agent uninstall <recipe> [--version <v>]`, `agent acp [--print-config
[<editor>]]` and `agent run`, beside the existing in-sandbox verbs.
`agent acp --print-config` SHALL output an editor ACP command snippet using the
resolved absolute `devcroft` executable path so GUI-launched editors with a
minimal `PATH` can invoke it directly. `up`'s `USAGE` SHALL list `--agent`,
`--agent-auth`, `--no-agent` and `--agent-from-config`. A verb run on the wrong side SHALL fail
at layer `config` naming the side it belongs to. All SHALL be listed in
`USAGE`. `status` SHALL show the selected agent, its version, its source
(embedded or user), its active auth mode, its selection provenance, its tier
(`artifact`), and that it is stale — printing a valid `devcroft up [--name <n>]
--recreate` command rooted at `project_root` — when the recorded selection would
now resolve to another `recipe_digest`, and SHALL say when the user
configuration's default differs from the recorded selection. `doctor` SHALL report, without
installing anything, whether the selected recipe supports this platform,
whether its object and pin are present, whether its selected authentication
mode is satisfied (without running a credential command), and, under `doctor
--probe-acp` on a running sandbox only, whether the ACP `initialize` handshake
succeeds cleanly.

#### Scenario: Status names the agent, auth mode and tier
- **WHEN** a sandbox with an agent is up
- **THEN** `status` prints the recipe name, its version, its active auth mode and `artifact`

#### Scenario: Stale agent prints valid up recreate command for named sandbox
- **WHEN** devcroft is upgraded to a changed recipe, or the user edits a user recipe, while sandbox `wt2` (created with `devcroft up --name wt2`) runs
- **THEN** `status wt2` prints that the agent is stale and names `devcroft up --name wt2 --recreate`

#### Scenario: Print-config emits GUI-safe absolute executable path
- **WHEN** `devcroft agent acp --print-config` runs
- **THEN** it prints a configuration snippet referencing the absolute path of `devcroft`

#### Scenario: Help lists the commands
- **WHEN** `devcroft --help` runs
- **THEN** `agent list`, `agent install`, `agent uninstall`, `agent acp` and `agent run` are listed as host commands

#### Scenario: Host verb run on the host
- **WHEN** `devcroft agent list` runs outside any sandbox
- **THEN** it succeeds rather than reporting that it is not inside a sandbox

### Requirement: Fleet refuses a selected agent until qualified
Until fleet consumes recipes, `fleet up` with an agent selected SHALL fail at
layer `config` saying fleet does not run recipes yet, rather than ignoring the
selection.

#### Scenario: Fleet with an agent
- **WHEN** the user configuration selects a recipe and `fleet up` runs
- **THEN** it fails naming the recipe and the missing support

## ADDED Requirements

### Requirement: Interactive agent commands and visibility

The host CLI SHALL provide `agent list`, `agent install <recipe> [--version <v>]`, `agent uninstall <recipe> [--version <v>]` and `agent run [<recipe>] [--sandbox <name>]`. `agent list` SHALL show available versions, source, platform, authentication modes, ACP capability and install/pin state. Help SHALL distinguish host and in-sandbox verbs and list the agent selection flags, `--agent-upgrade` included. Wrong-side use SHALL fail at layer `config`. `status` SHALL show the selected agent, version, source, selection provenance, auth mode, `artifact` tier, changed global default, and recipe staleness whenever the recorded selection would now resolve to a different full `recipe_digest` (covering artifacts, entrypoints, `launch.env`, `reserved_env`, `hosts`, `hooks`, `requires`, `state_paths` and `auth_mode`, not only `object_id`), printing a copyable `up --recreate --agent-upgrade` command prefixed with `cd <quoted project_root> && ` whenever canonical `cwd`'s nearest manifest root differs from `project_root` and `--name <n>` whenever `sandbox_name != manifest.sandbox.name`. `status` SHALL keep its positional `[name]` and accept `--sandbox <name>` as the same selector, since the shared resolver's remedies name `--sandbox`. `doctor` SHALL report platform, object, pin and auth prerequisites without installing or acquiring a credential, describing a credential only as `present in host environment`, `configured (not run)` or `missing`, never as satisfied.

#### Scenario: Status names the agent, auth mode and tier
- **WHEN** a sandbox with an agent is up
- **THEN** `status` prints the recipe name, its version, its active auth mode and `artifact`

#### Scenario: Stale agent prints valid up recreate command for named sandbox from any directory
- **WHEN** devcroft is upgraded to a recipe whose `hosts` or `launch.env` changed (even with the same binary `object_id`), while sandbox `wt2` (created at `/repo` with `devcroft up --name wt2`) runs, and `devcroft status wt2` runs outside `/repo`
- **THEN** `status wt2` prints that the agent is stale and names `cd /repo && devcroft up --name wt2 --recreate --agent-upgrade`

#### Scenario: Help lists the commands
- **WHEN** `devcroft --help` runs
- **THEN** `agent list`, `agent install`, `agent uninstall` and `agent run` are listed as host commands

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

## ADDED Requirements

### Requirement: Agent selection in the user configuration
The user configuration SHALL accept an `[agent]` table with `name`, naming a
recipe, an optional `version`, and an optional `auth` (a mode qualified for that recipe and platform). Unknown keys or unsupported `auth` values SHALL fail at layer
`config`. `devcroft up --agent <recipe>` (and `--agent-auth <mode>`), `--no-agent`
and `--agent-from-config` SHALL choose explicitly (`--agent`, `--no-agent` and
`--agent-from-config` being mutually exclusive, and `--agent-auth` without a
resulting agent failing at layer `config`). For a brand-new sandbox (no `Meta`
on disk), an explicit flag or `[agent]` SHALL be recorded in `Meta`. For any
existing sandbox (`Meta` on disk, whether running or stopped), changing the
effective selection away from `Meta`'s recorded state — including adding an
agent to an `Unset` or `Disabled` sandbox — SHALL require `--recreate`, failing
at layer `config` without it. Without flags, plain `up`, auto-up and `up
--recreate` SHALL keep a recorded `Selected` or `Disabled` choice; for a sandbox
with `Unset`, plain `up` and auto-up SHALL keep `Unset` (no agent) while `up
--recreate` SHALL read and record `[agent]`. Switching to another recipe SHALL
NOT carry over the previous recipe's `version` or `auth` mode, and SHALL consult
`[agent]`'s `version` and `auth` when `[agent]` names the new recipe. Plain `up`
SHALL run the recorded recipe even if this devcroft no longer ships it;
`--recreate` SHALL re-resolve the recorded selection, keeping a requested
version for that same recipe. The project's `devcroft.toml` SHALL NOT be read
for agent selection, an `[agent]` table there SHALL fail at layer `config`
pointing to the user configuration, and selecting an agent SHALL NOT change the
project's provider or its resolved environment.

#### Scenario: Flag overrides configuration
- **WHEN** the user configuration names `fixture-agent` and `up --agent opencode` is run
- **THEN** the sandbox gets the `opencode` recipe and records that explicit selection

#### Scenario: Opting out survives recreate
- **WHEN** the user configuration names a recipe and the user runs `up --no-agent`, then `down`, then `up`, then `up --recreate`
- **THEN** the sandbox has no agent rules, hosts or broker after each

#### Scenario: Recreate keeps the recipe and the mode
- **WHEN** the configuration's default is `fixture-agent` and a sandbox brought up with `--agent opencode --agent-auth api-key` is recreated without flags
- **THEN** it still runs `opencode` under `api-key`

#### Scenario: Adopting the default
- **WHEN** that sandbox is brought up with `up --agent-from-config --recreate`
- **THEN** it runs `fixture-agent` and records that selection

#### Scenario: Changed default
- **WHEN** the user edits `[agent] name` or `version` and runs plain `up` or `up --recreate` on a sandbox with a recorded selection
- **THEN** its agent is unchanged, and `status` says the default differs and names `--agent-from-config`

#### Scenario: Sandbox that never had a selection
- **WHEN** a sandbox was created before `[agent]` was configured, stopped with `down`, and plain `up`, `up --agent fixture-agent` (without `--recreate`), then `up --recreate` run
- **THEN** plain `up` adds no agent or host, `up --agent fixture-agent` without `--recreate` fails naming `--recreate`, and `up --recreate` applies and records `[agent]`

#### Scenario: Switching recipes drops the old version
- **WHEN** a sandbox pinned to version 1 of a fixture recipe is recreated with a different recipe configured at version 2
- **THEN** it selects version 2 and the authentication mode applicable to the new recipe

#### Scenario: Recorded recipe no longer shipped
- **WHEN** a sandbox is down, devcroft is upgraded to a release without its recipe version, and plain `up` runs
- **THEN** it runs the recorded recipe, re-materialized if its object is gone

#### Scenario: Provider untouched
- **WHEN** a sandbox is brought up with and then without an agent
- **THEN** the provider's fingerprint and captured environment are identical in both

#### Scenario: Misplaced table
- **WHEN** a project's `devcroft.toml` contains `[agent]`
- **THEN** loading it fails at layer `config` naming the user configuration file

#### Scenario: Misspelled key
- **WHEN** the user configuration contains `[agent] nmae = "fixture-agent"`
- **THEN** it fails at layer `config` naming the key

### Requirement: PATH order with tools and agent
With both a tooling layer and an agent selected, `PATH` SHALL be the
project's entries, then the tooling layer, then the agent's launcher
directory, then the base runtime fallback.

#### Scenario: Both layers
- **WHEN** `[tools]` and `[agent]` are both configured
- **THEN** `PATH` inside the sandbox lists the project's entries, the tooling layer and the launcher directory, in that order

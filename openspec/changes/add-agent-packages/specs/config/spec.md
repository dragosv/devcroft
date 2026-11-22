## ADDED Requirements

### Requirement: Agent selection in the user configuration
The user configuration SHALL accept an `[agent]` table with `name`, naming a
recipe, and an optional `version`. Unknown keys SHALL fail at layer `config`.
`devcroft up --agent <recipe>` SHALL select one for a single invocation,
taking precedence over the table, and `devcroft up --no-agent` SHALL select
none. The project's `devcroft.toml` SHALL NOT be read for agent selection, an
`[agent]` table there SHALL fail at layer `config` pointing to the user
configuration, and selecting an agent SHALL NOT change the project's provider
or its resolved environment.

#### Scenario: Flag overrides configuration
- **WHEN** the user configuration names `claude-code` and `up --agent opencode` is run
- **THEN** the sandbox gets the `opencode` recipe

#### Scenario: Opting out for one project
- **WHEN** the user configuration names a recipe and `up --no-agent` is run
- **THEN** the sandbox has no agent rules, hosts or broker

#### Scenario: Provider untouched
- **WHEN** a sandbox is brought up with and then without an agent
- **THEN** the provider's fingerprint and captured environment are identical in both

#### Scenario: Misplaced table
- **WHEN** a project's `devcroft.toml` contains `[agent]`
- **THEN** loading it fails at layer `config` naming the user configuration file

#### Scenario: Misspelled key
- **WHEN** the user configuration contains `[agent] nmae = "claude-code"`
- **THEN** it fails at layer `config` naming the key

### Requirement: PATH order with tools and agent
With both a tooling layer and an agent selected, `PATH` SHALL be the
project's entries, then the tooling layer, then the agent's launcher
directory.

#### Scenario: Both layers
- **WHEN** `[tools]` and `[agent]` are both configured
- **THEN** `PATH` inside the sandbox lists the project's entries, the tooling layer and the launcher directory, in that order

## ADDED Requirements

### Requirement: Agent commands and visibility
The CLI SHALL provide the host-side verbs `devcroft agent list` (each recipe
with its versions, ACP mode, platforms, authentication variable and whether
it is installed), `agent install <recipe>`, `agent acp` and `agent run`,
beside the existing in-sandbox verbs. A verb run on the wrong side SHALL fail
at layer `config` naming the side it belongs to. All SHALL be listed in
`USAGE`. `status` SHALL show the selected agent, its version, its source
(embedded or user), its tier, and that it is stale when the configuration no
longer matches the running sandbox. `doctor` SHALL report, without installing
anything, whether the selected recipe supports this platform, whether its
object is present and whether its authentication variable is set.

#### Scenario: Status names the agent and its tier
- **WHEN** a sandbox with an agent is up
- **THEN** `status` prints the recipe name, its version and `artifact`

#### Scenario: Stale agent
- **WHEN** the user changes `[agent] version` while the sandbox runs
- **THEN** `status` prints that the agent is stale and names `devcroft up --recreate`

#### Scenario: Help lists the commands
- **WHEN** `devcroft --help` runs
- **THEN** `agent list`, `agent install`, `agent acp` and `agent run` are listed as host commands

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

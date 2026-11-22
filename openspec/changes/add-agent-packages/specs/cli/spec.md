## ADDED Requirements

### Requirement: Agent commands and visibility
The CLI SHALL provide `devcroft agent list`, showing each recipe with its
version, ACP mode and platforms, and `devcroft agent acp [name]`. Both SHALL
be listed in `USAGE`. `status` SHALL show the selected agent, its recipe
version and its tier. `doctor` SHALL report, without installing anything,
whether the selected recipe supports this platform and whether its object is
present.

#### Scenario: Status names the agent and its tier
- **WHEN** a sandbox with an agent is up
- **THEN** `status` prints the recipe name, its version and `artifact`

#### Scenario: Help lists the commands
- **WHEN** `devcroft --help` runs
- **THEN** `agent list` and `agent acp` are listed

## ADDED Requirements

### Requirement: Agent selection in the user configuration
The user configuration SHALL accept an `[agent]` table with `name`, naming a
recipe, and `devcroft up --agent <recipe>` SHALL select one for a single
invocation, taking precedence over the table. The project's `devcroft.toml`
SHALL NOT be read for agent selection, and selecting an agent SHALL NOT
change the project's provider or its resolved environment.

#### Scenario: Flag overrides configuration
- **WHEN** the user configuration names `claude` and `up --agent opencode` is run
- **THEN** the sandbox gets the `opencode` recipe

#### Scenario: Provider untouched
- **WHEN** a sandbox is brought up with and then without an agent
- **THEN** the provider's fingerprint and captured environment are identical in both

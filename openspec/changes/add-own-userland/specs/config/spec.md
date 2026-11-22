## ADDED Requirements

### Requirement: The runtime table
`devcroft.toml` SHALL accept an optional `[runtime]` table with one key,
`generation`, a positive integer. Any other key in `[runtime]` SHALL fail
validation at layer `config`, exit code 2, naming the key. The table governs
which devcroft runtime generation the project requires; it SHALL NOT change any
provider setting.

#### Scenario: A pinned generation parses
- **WHEN** `devcroft.toml` contains `[runtime]` with `generation = 1`
- **THEN** the manifest validates and the pinned generation is 1

#### Scenario: An unknown key
- **WHEN** `[runtime]` contains `shell = "bash"`
- **THEN** validation fails at layer `config` with exit code 2 naming `shell`

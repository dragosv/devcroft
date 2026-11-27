## ADDED Requirements

### Requirement: Credential command for up

Only the user configuration SHALL accept `[agent.credentials.<recipe>] command` as a non-empty argument vector. When the host key variable is unset or empty (`""`), `up` SHALL run this command once with a deadline, without a shell, trim trailing newline characters, and deliver its value according to the selected mode. If the command exits non-zero, exceeds the deadline, or produces an empty value, `up` SHALL fail at layer `config` naming the command entry. Preflight, doctor and `agent acp` SHALL NOT execute it. The value SHALL never be logged or recorded.

#### Scenario: Key command with an unset or empty host variable
- **WHEN** `up` runs from a shell where the key is unset or set to `""`, and the recipe has a configured credential command
- **THEN** the sandbox comes up and the broker uses the command's value, which appears in no log, policy or `Meta`

#### Scenario: Empty or failing credential command refuses startup
- **WHEN** `[agent.credentials.<recipe>] command` exits non-zero or prints only whitespace/newlines
- **THEN** `up` fails at layer `config` naming the command entry without starting the keeper

#### Scenario: Project declares a credential command
- **WHEN** `devcroft.toml` contains an agent credential command
- **THEN** loading it fails at layer `config` naming the user configuration file

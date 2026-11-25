## ADDED Requirements

### Requirement: Mode flags and visibility
`up` SHALL accept `--agent-mode <1|2>` and `--quarantine-agent-state`, listed in
`USAGE`. `--quarantine-agent-state` SHALL fail with exit code 2 unless `up` is
entering mode 2 for a recipe recorded in `Meta.mode1_recipes` whose
`state_paths` exist in the project `HOME`. `status` SHALL show the mode and any
quarantined state, and `rm` SHALL name that state before removing it.

#### Scenario: Flag when no prior Mode 1 state is pending quarantine
- **WHEN** `up --quarantine-agent-state` runs on a sandbox already in mode 2 with no pending `mode1_recipes` state
- **THEN** it fails with exit code 2 and moves nothing

#### Scenario: Quarantine shown
- **WHEN** a sandbox has quarantined state
- **THEN** `status` names its directory

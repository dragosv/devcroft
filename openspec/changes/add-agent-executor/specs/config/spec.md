## ADDED Requirements

### Requirement: Agent mode in the user configuration
`[agent]` in the user configuration SHALL accept `mode`, 1 (the default) or 2,
for brand-new sandboxes (and `--recreate` of sandboxes with `Unset`), and
`up --agent-mode <1|2>` SHALL choose explicitly. Changing the mode of an
existing sandbox SHALL require `--recreate`. The mode SHALL be recorded with the
agent selection and kept across `down`, `up`, `up --recreate` and recipe
switches (unless `--agent-mode` or `--agent-from-config` overrides it), so
switching a mode 2 sandbox to an ineligible recipe or auth mode fails closed
naming `--agent-mode 1` rather than silently downgrading.

#### Scenario: Default
- **WHEN** `[agent]` names a recipe without `mode`
- **THEN** a new sandbox runs in mode 1

#### Scenario: Recreate keeps the mode
- **WHEN** a sandbox brought up with `--agent-mode 2` is recreated without flags
- **THEN** it runs in mode 2

#### Scenario: Unknown mode
- **WHEN** `[agent] mode = 3`
- **THEN** loading it fails at layer `config` naming the key

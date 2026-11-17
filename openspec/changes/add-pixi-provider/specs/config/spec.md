## ADDED Requirements

### Requirement: Pixi provider and environment selection
The system SHALL accept `env.provider = "pixi"` without changing existing
provider values or defaults. It SHALL accept optional `[env.pixi]` keys
`manifest` and `environment`, defaulting to the sole eligible root manifest
and the named `default` environment. Unknown keys and `[env.pixi]` under
another provider SHALL be config errors. Selection SHALL NOT depend on any
ambient `PIXI_*` value, and resolution, recorded state and staleness SHALL use
the same selection.

#### Scenario: Explicit named environment
- **WHEN** the manifest selects Pixi and `[env.pixi] environment = "test"`
- **THEN** resolution, recorded state and staleness all identify the `test` environment

#### Scenario: Configuration belongs to another provider
- **WHEN** `[env.pixi]` is present while `env.provider` is `flox`
- **THEN** parsing fails at layer `config`, exit code 2, naming the section

#### Scenario: Ambient selection is ignored
- **WHEN** the caller's shell sets `PIXI_ENVIRONMENT_NAME` to another environment
- **THEN** the environment selected by `devcroft.toml` is the one resolved

### Requirement: Pixi workspace discovery is unambiguous and bounded
An eligible manifest SHALL be `pixi.toml` or a `pyproject.toml` with
`[tool.pixi]`. Without explicit selection, multiple eligible root files SHALL
be refused. An explicit path SHALL resolve to a regular file within the
canonical project root. Missing or invalid workspace definitions SHALL be
provider errors before installation or evaluation.

#### Scenario: Unrelated Python project
- **WHEN** only a `pyproject.toml` without `[tool.pixi]` exists
- **THEN** it is not treated as a Pixi workspace

#### Scenario: Ambiguous workspace
- **WHEN** both root manifests are eligible and neither is selected
- **THEN** the provider names both files and asks for an explicit `manifest`

#### Scenario: Escaping selector
- **WHEN** a manifest selector traverses or resolves through a symlink outside the project
- **THEN** it is refused before any Pixi subprocess starts

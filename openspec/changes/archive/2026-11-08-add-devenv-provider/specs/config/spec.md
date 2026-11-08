# config Delta Specification (add-devenv-provider)

## ADDED Requirements

### Requirement: devenv provider value
The system SHALL accept `devenv` as a value for `env.provider`. The name
has no aliases — like `devbox` and unlike `nix`, devenv is known by
exactly one name, and the system SHALL NOT invent one. The default
provider remains `flox`.

#### Scenario: Canonical name accepted
- **WHEN** the manifest declares `provider = "devenv"`
- **THEN** validation succeeds and the resolved config carries provider
  `devenv`

#### Scenario: No alias invented
- **WHEN** the manifest declares a near-miss such as `provider = "dev-env"`
- **THEN** validation fails with exit code 2 rather than normalizing it to
  `devenv`

#### Scenario: Default unchanged
- **WHEN** the manifest omits `[env]` entirely
- **THEN** the provider defaults to `flox`, exactly as before this change

#### Scenario: Adding a provider does not change any other manifest
- **WHEN** a manifest that does not name `devenv` is parsed and compiled
- **THEN** the resulting policy is byte-identical to what it produced
  before this change

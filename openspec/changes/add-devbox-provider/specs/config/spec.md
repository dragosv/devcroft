# config Delta Specification (add-devbox-provider)

## ADDED Requirements

### Requirement: devbox provider value
The system SHALL accept `devbox` as a value for `env.provider`. The name
has no aliases — unlike `nix`, which normalizes `flake`/`flakes`, devbox
is known by exactly one name, and the system SHALL NOT invent one. The
default provider remains `flox`.

#### Scenario: Canonical name accepted
- **WHEN** the manifest declares `provider = "devbox"`
- **THEN** validation succeeds and the resolved config carries provider
  `devbox`

#### Scenario: Default unchanged
- **WHEN** the manifest omits `[env]` entirely
- **THEN** the provider defaults to `flox`, exactly as before this change

#### Scenario: Adding a provider does not change any other manifest
- **WHEN** a manifest that does not name `devbox` is parsed and compiled
- **THEN** the resulting policy is byte-identical to what it produced
  before this change

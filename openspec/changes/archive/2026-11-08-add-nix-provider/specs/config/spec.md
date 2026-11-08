# config Delta Specification (add-nix-provider)

## ADDED Requirements

### Requirement: nix provider value
The system SHALL accept `nix` as a value for `env.provider`, and SHALL
normalize the aliases `flake` and `flakes` to `nix` at parse time so
exactly one canonical name reaches provider dispatch, `status` output,
and policy rule origins. The default provider remains `flox`.

#### Scenario: Canonical name accepted
- **WHEN** the manifest declares `provider = "nix"`
- **THEN** validation succeeds and the resolved config carries provider
  `nix`

#### Scenario: Aliases normalize
- **WHEN** the manifest declares `provider = "flakes"`
- **THEN** validation succeeds and the resolved config carries provider
  `nix`, with `status` and `policy --render` showing `nix`, never the
  alias

#### Scenario: Default unchanged
- **WHEN** the manifest omits `[env]` entirely
- **THEN** the provider defaults to `flox`, exactly as before this change

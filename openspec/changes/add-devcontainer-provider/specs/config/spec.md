## ADDED Requirements

### Requirement: devcontainer provider value
The system SHALL accept `devcontainer` as a value for `env.provider`,
with no aliases (`docker`, `container`, `oci` are not accepted and the
system SHALL NOT invent one). The default provider remains `flox`.
Parsing SHALL succeed on every platform; refusing on a non-Linux host is
resolution's job, so that a manifest committed by a Linux team parses
for a macOS reader and fails with the platform reason rather than as a
config error.

#### Scenario: Canonical name accepted
- **WHEN** the manifest declares `provider = "devcontainer"`
- **THEN** validation succeeds and the resolved config carries provider
  `devcontainer`

#### Scenario: Default unchanged
- **WHEN** the manifest omits `[env]` entirely
- **THEN** the provider defaults to `flox`, exactly as before this change

#### Scenario: Adding a provider does not change any other manifest
- **WHEN** a manifest that does not name `devcontainer` is parsed and compiled
- **THEN** the resulting policy is byte-identical to what it produced
  before this change

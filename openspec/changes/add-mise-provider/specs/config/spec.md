# config Specification

## Purpose

Accept `mise` as a valid provider enum value in the manifest. Delta
against `openspec/specs/config/` (currently empty — no change has archived
yet), so this is additive, not a modification of an existing requirement.

## ADDED Requirements

### Requirement: mise as a provider enum value
The system SHALL accept `mise` as a valid value for `[env].provider`, in
addition to the MVP's `flox`. This lifts the MVP-era rejection (env-provider
spec's "Planned provider rejected in MVP" scenario for `provider = "mise"`)
now that the provider is implemented.

#### Scenario: mise provider selection
- **WHEN** the manifest sets `provider = "mise"`
- **THEN** validation accepts it, where MVP validation would have rejected
  it with an exit code 2 "planned but not yet implemented" message

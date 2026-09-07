# config Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: swift provider value
The system SHALL accept `swift` as a value for `env.provider`. The name
has no aliases — `swiftpm` and `spm` SHALL NOT be accepted, because a
provider that fails the qualification test should be nameable in exactly
one way that a reader can search for. The default provider remains
`flox`.

#### Scenario: Canonical name accepted
- **WHEN** the manifest declares `provider = "swift"`
- **THEN** validation succeeds and the resolved config carries provider
  `swift`

#### Scenario: No aliases are invented
- **WHEN** the manifest declares `provider = "swiftpm"` or `"spm"`
- **THEN** validation fails with the unknown-provider error

#### Scenario: Default unchanged
- **WHEN** the manifest omits `[env]` entirely
- **THEN** the provider defaults to `flox`, exactly as before this change

#### Scenario: Adding a provider does not change any other manifest
- **WHEN** a manifest that does not name `swift` is parsed and compiled
- **THEN** the resulting policy is byte-identical to what it produced
  before this change

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

### Requirement: The evidence gate is opt-in, in the manifest
The system SHALL accept `[env] require_native_apple_evidence = true`,
default `false`, which turns `up`'s advice that a closure-tier provider
would serve the project into a refusal at layer `provider`. The key
SHALL be rejected at layer `config` under any provider but `swift`,
since it would otherwise be silently inert in a committed file.

#### Scenario: Set under swift
- **WHEN** the manifest declares `provider = "swift"` and the key `true`
- **THEN** validation succeeds and `up` refuses a portable package

#### Scenario: Set under another provider
- **WHEN** the manifest declares `provider = "flox"` and the key `true`
- **THEN** validation fails with exit code 2, naming the key and that it
  applies to `swift` only

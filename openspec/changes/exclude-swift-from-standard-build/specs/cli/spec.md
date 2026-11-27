## ADDED Requirements

### Requirement: CLI help and init reflect the standard provider set
In standard builds, `devcroft --help`, `devcroft init --help`, `devcroft init` and `devcroft doctor` SHALL describe only providers included in the standard distribution. Running `devcroft init` in a directory containing `Package.swift` and no other provider files SHALL refuse to guess `swift` and SHALL direct the user to initialize a supported declarative provider.

#### Scenario: Init on a bare Package.swift repository
- **WHEN** `devcroft init` runs in a directory containing only `Package.swift` using a standard build
- **THEN** it does not write `provider = "swift"` and explains how to initialize a supported provider

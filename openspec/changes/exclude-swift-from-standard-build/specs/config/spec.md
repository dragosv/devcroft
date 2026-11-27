## ADDED Requirements

### Requirement: Standard builds refuse native swift explicitly at layer config
In standard builds (without the non-default research feature), `env.provider = "swift"` SHALL fail validation at layer `config` (exit code 2) with a message explaining that the native macOS Swift provider is excluded because it violates devcroft's non-host shell and base-userland invariant, and naming closure-tier providers (`flox`, `nix`) for portable Swift packages. `swiftpm` and `spm` SHALL remain `Unknown`, and devcroft SHALL NOT fall back to another provider.

#### Scenario: Manifest requesting swift fails closed in standard build
- **WHEN** `devcroft.toml` sets `env.provider = "swift"` and `devcroft up` runs using the standard build
- **THEN** `up` exits with code 2 at layer `config` naming the exclusion, creates no state directory, and executes neither `xcrun` nor `swift`

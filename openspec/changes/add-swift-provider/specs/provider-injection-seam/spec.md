## MODIFIED Requirements

### Requirement: The seam is not reachable from a manifest or the published binary

`ProviderKind` SHALL admit exactly the shipped providers — `flox`, `nix`,
`devbox`, `devenv` and `swift` — and `config::parse` SHALL continue to
reject every other `env.provider` value. The seam SHALL live behind a
non-default Cargo feature and SHALL NOT be compiled into a default build.

devcroft's standing position is that there is no non-reproducible mode and no
passthrough provider. A test seam that could be named in a `devcroft.toml`
would be exactly that exception, whatever it was called. (The list was
"exactly `flox`, `nix` and `devbox`" when written; it names the shipped
set, not a fixed number, and grows with the providers.)

#### Scenario: A manifest naming a test provider

- **WHEN** a `devcroft.toml` sets `env.provider` to anything outside the
  shipped providers
- **THEN** parsing fails, as it does today
- **AND** the failure distinguishes "not supported" from "out of scope by design"

#### Scenario: A default build

- **WHEN** the crate is built without the test-support feature
- **THEN** the seam is absent from the binary
- **AND** no code path can select a fixture at runtime

## ADDED Requirements

### Requirement: The matrix has a swift row

The runtime fixture SHALL offer a `swift` row — a dependency-free SwiftPM
package with an unguarded Apple import and a manifest naming the
provider — selectable as `DEVCROFT_TEST_PROVIDER=swift` and included in
`all`, unavailable by name off macOS or where no toolchain answers
`swift -print-target-info`. Its capabilities SHALL be: no services, no
activation hook, external utilities present (the host's userland is a
provider grant), CLI-drivable, staleness on `Package.swift`. Every
neutral lifecycle test SHALL pass on it unchanged: the artifact tier is
held to the same contract as the closure tier, and this row is what
holds it.

#### Scenario: The lifecycle matrix runs under swift

- **WHEN** `DEVCROFT_TEST_PROVIDER=swift` on a macOS host with a toolchain
- **THEN** every `for_each_row` test reports `swift ok` — up, down,
  recreate, status, stale, exec, shell, worktree identity

#### Scenario: CI's macOS leg selects `all`

- **WHEN** the provider-free CI leg runs on a macOS runner
- **THEN** it selects `all`, the swift row and the nix-free row run, the
  store-backed rows skip by name, and the leg fails only if every row
  skipped

## ADDED Requirements

### Requirement: init detects a dev container definition
The system SHALL, in `devcroft init`, detect `.devcontainer/devcontainer.json`
or `.devcontainer.json` and select `devcontainer` only when no closure
provider is present — after flox, devbox, devenv and a bare flake, and
before `swift` where that provider exists — and SHALL say at selection
time that the image tier is what was chosen and why.

#### Scenario: Init on a project with only a dev container
- **WHEN** the directory has `.devcontainer/devcontainer.json` and no
  flox, devbox, devenv or flake definition
- **THEN** `init` writes `provider = "devcontainer"` and prints the tier
  and the closure-tier alternative

#### Scenario: A closure provider wins
- **WHEN** the directory has both a dev container definition and a
  `flake.nix`
- **THEN** `init` selects `nix`

### Requirement: doctor reports the registry, the rootfs store and the digest
The system SHALL, in `devcroft doctor` for a `devcontainer` project,
report whether the image's registry is reachable, whether the rootfs
store is writable, and — where the project resolves — whether its digest
is materialized (in which case no network is needed). It SHALL NOT
probe for or mention a container runtime, since none is used. Each
failure SHALL name the fix.

#### Scenario: Registry unreachable, digest already present
- **WHEN** the digest is materialized and the registry cannot be reached
- **THEN** `doctor` reports the store as complete for this project and
  the registry as unreachable, and says `up` will succeed offline

#### Scenario: A registry needing a credential helper
- **WHEN** the image's registry requires a credential helper devcroft
  does not drive
- **THEN** `doctor` names it as unsupported in this version and points at
  a token in the registry's standard auth file as the alternative

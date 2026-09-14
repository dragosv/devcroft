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

### Requirement: doctor reports the OCI runtime and the rootfs store
The system SHALL, in `devcroft doctor` for a `devcontainer` project,
report whether a usable OCI runtime is present and which, that it is
required at provisioning only, whether the rootfs store is writable,
and — where the project resolves — whether its digest is materialized.
Each failure SHALL name the fix.

#### Scenario: Docker present but not usable by this user
- **WHEN** `docker` is on `PATH` and the daemon refuses the user
- **THEN** `doctor` reports the runtime as present and unusable, names
  the group or rootless mode as the fix, and names `podman` as the
  alternative

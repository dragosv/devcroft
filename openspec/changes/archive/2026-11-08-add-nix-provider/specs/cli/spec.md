# cli Delta Specification (add-nix-provider)

## MODIFIED Requirements

### Requirement: init
The system SHALL provide `devcroft init` which detects an existing flox
environment (`.flox/`), an existing nix flake (`flake.nix`),
single-ecosystem toolchain pins (`rust-toolchain.toml`, `.nvmrc`,
`.python-version`), and the project language, generates a minimal
`devcroft.toml` with commented-out common options, and never overwrites
an existing manifest without `--force`.

#### Scenario: Init on a flox project
- **WHEN** `.flox/` exists in the project root
- **THEN** the generated manifest sets `provider = "flox"` and the sandbox
  name defaults to the directory slug

#### Scenario: Init on a nix flake project
- **WHEN** `flake.nix` exists in the project root and no `.flox/` does
- **THEN** the generated manifest sets `provider = "nix"`
- **AND** if `flake.lock` is absent, init prints the next step: run
  `nix flake lock` before `up`

#### Scenario: Both flox and a flake present
- **WHEN** `.flox/` and `flake.nix` both exist in the project root
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init states in one line that a flake was also found and that
  `provider = "nix"` is available

#### Scenario: Init on a Rust project with a pinned toolchain
- **WHEN** `rust-toolchain.toml` exists and neither `.flox/` nor
  `flake.nix` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init offers to generate a flox manifest honoring the pinned
  channel (via fenix or rust-overlay) together with a C toolchain,
  explaining in one line that rustup alone cannot provide a complete
  build environment

#### Scenario: Init on a Node project with a pinned version
- **WHEN** `.nvmrc` exists and neither `.flox/` nor `flake.nix` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init explains in one line that nvm alone cannot provide a
  complete build environment, and to run `flox init` and pin the Node.js
  version from `.nvmrc` before `up`

#### Scenario: Init on a Python project with a pinned version
- **WHEN** `.python-version` exists and neither `.flox/` nor `flake.nix`
  does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init explains in one line that pyenv alone cannot provide a
  complete build environment, and to run `flox init` and pin the Python
  version from `.python-version` before `up`

#### Scenario: An existing environment supersedes any toolchain pin
- **WHEN** `.flox/` or `flake.nix` exists alongside one or more of
  `rust-toolchain.toml`, `.nvmrc`, `.python-version`
- **THEN** init reports the project as ready for `up` (or, for a flake
  without `flake.lock`, one `nix flake lock` away from it) and does not
  print toolchain-pin advice — a real declarative environment supersedes
  advice about a pin it would otherwise just be a fallback for

#### Scenario: Disambiguate a real name collision
- **WHEN** the directory slug matches the default name of an already-known
  sandbox (state exists for that name, recorded for a different project
  root — e.g. two unrelated projects both named `api`)
- **THEN** init appends a short suffix derived from the project's absolute
  path to keep the two disjoint, rather than silently generating a name
  that would collide with the other project's state dir and control
  socket
- **AND** re-running init in the *same* project (its own state, its own
  project root) is not treated as a collision and keeps the plain slug

#### Scenario: Init without an environment
- **WHEN** neither `.flox/` nor `flake.nix` exists in the project root
- **THEN** the manifest is still generated with `provider = "flox"`
- **AND** init prints the next step: run `flox init` before `up`

### Requirement: doctor
The system SHALL provide `devcroft doctor` reporting, per requirement:
backend binary presence and version-range compatibility, kernel capability
(Landlock ABI level / Seatbelt availability), provider binaries (including,
for nix, whether flake commands are enabled), ssh config managed-section
state, and which manifest aspects would be degraded on this host. Output
SHALL be actionable (each failure names the fix).

#### Scenario: Unsupported nono version
- **WHEN** the installed nono version is outside the tested range
- **THEN** `doctor` reports FAIL for the backend with the expected range and
  the install command

#### Scenario: nix present but flakes disabled
- **WHEN** `nix` is on PATH but flake commands are rejected
  (experimental features disabled)
- **THEN** `doctor` reports FAIL for the nix provider and names the
  configuration change that enables flakes

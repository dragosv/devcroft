# cli Specification

## Purpose

Define the command surface, name resolution, initialization, diagnostics,
and error contract shared by all capabilities.

## ADDED Requirements

### Requirement: Command surface (MVP)
The system SHALL provide exactly these subcommands in MVP: `init`, `up`,
`down`, `rm`, `status`, `logs`, `ps`, `shell`, `exec`, `ssh`, `proxy`,
`ssh-config`, `policy`, `why`, `doctor`. Anything else is post-MVP.

#### Scenario: ps lists all sandboxes
- **WHEN** the user runs `devcroft ps`
- **THEN** every sandbox with existing state is listed with name, keeper
  health, session count, and project root

### Requirement: Name resolution
The system SHALL resolve the target sandbox as: explicit name argument if
given; otherwise the sandbox whose project root contains the cwd; otherwise
fail with exit code 2 listing known sandboxes.

#### Scenario: Ambiguity is impossible by construction
- **WHEN** two manifests exist in the ancestor chain
- **THEN** the nearest one wins, and `status` names which manifest is active

### Requirement: init
The system SHALL provide `devcroft init` which detects an existing flox
environment (`.flox/`), single-ecosystem toolchain pins
(`rust-toolchain.toml`, `.nvmrc`, `.python-version`), and the project
language, generates a minimal `devcroft.toml` with commented-out common
options, and never overwrites an existing manifest without `--force`.

#### Scenario: Init on a flox project
- **WHEN** `.flox/` exists in the project root
- **THEN** the generated manifest sets `provider = "flox"` and the sandbox
  name defaults to the directory slug

#### Scenario: Init on a Rust project with a pinned toolchain
- **WHEN** `rust-toolchain.toml` exists and no `.flox/` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init offers to generate a flox manifest honoring the pinned
  channel (via fenix or rust-overlay) together with a C toolchain,
  explaining in one line that rustup alone cannot provide a complete
  build environment

#### Scenario: Init on a Node project with a pinned version
- **WHEN** `.nvmrc` exists and no `.flox/` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init explains in one line that nvm alone cannot provide a
  complete build environment, and to run `flox init` and pin the Node.js
  version from `.nvmrc` before `up`

#### Scenario: Init on a Python project with a pinned version
- **WHEN** `.python-version` exists and no `.flox/` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init explains in one line that pyenv alone cannot provide a
  complete build environment, and to run `flox init` and pin the Python
  version from `.python-version` before `up`

#### Scenario: An existing flox environment supersedes any toolchain pin
- **WHEN** `.flox/` exists alongside one or more of `rust-toolchain.toml`,
  `.nvmrc`, `.python-version`
- **THEN** init reports the project as ready for `up` and does not print
  toolchain-pin advice — a real flox environment supersedes advice about a
  pin it would otherwise just be a fallback for

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
- **WHEN** no `.flox/` exists in the project root
- **THEN** the manifest is still generated with `provider = "flox"`
- **AND** init prints the next step: run `flox init` before `up`

### Requirement: doctor
The system SHALL provide `devcroft doctor` reporting, per requirement:
backend binary presence and version-range compatibility, kernel capability
(Landlock ABI level / Seatbelt availability), provider binaries, ssh config
managed-section state, and which manifest aspects would be degraded on this
host. Output SHALL be actionable (each failure names the fix).

#### Scenario: Unsupported nono version
- **WHEN** the installed nono version is outside the tested range
- **THEN** `doctor` reports FAIL for the backend with the expected range and
  the install command

### Requirement: Error contract
The system SHALL use stable exit codes: 0 success, 1 runtime failure,
2 usage/config error, 3 environment/provider failure, 4 backend failure,
5 keeper/connection failure. Every error message SHALL name its layer
(config | provider | backend | keeper | ssh).

#### Scenario: Distinguishable failure layers
- **WHEN** nono is missing versus the manifest is invalid
- **THEN** exit codes are 4 versus 2 and the layer prefix differs

### Requirement: Non-interactive safety
The system SHALL never prompt when stdout is not a tty; destructive
operations (`rm`, `up --recreate`) require `--yes` in non-interactive mode.

#### Scenario: rm in a script
- **WHEN** `devcroft rm myproj` runs with stdout piped and no `--yes`
- **THEN** it fails with exit code 2 and no state is removed

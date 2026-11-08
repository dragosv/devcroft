# cli Delta Specification (add-devbox-provider)

## MODIFIED Requirements

### Requirement: init
The system SHALL provide `devcroft init` which detects an existing flox
environment (`.flox/`), an existing devbox project (`devbox.json`), an
existing nix flake (`flake.nix`), single-ecosystem toolchain pins
(`rust-toolchain.toml`, `.nvmrc`, `.python-version`), and the project
language, generates a minimal `devcroft.toml` with commented-out common
options, and never overwrites an existing manifest without `--force`.

Where several environments are present, detection SHALL apply a fixed,
documented order — flox, then devbox, then a bare flake — and SHALL
state in one line which others were found and remain available.

The order is a deterministic tiebreak, not a judgement that the losers
are derived artifacts. Each of the three is a genuine, hand-maintained
environment definition, and a project carrying two of them has made a
choice devcroft cannot infer. What matters for correctness is that the
result is predictable and that the alternatives are named, so a user who
wanted the other one can see it and set `env.provider` themselves.

(An earlier draft justified ranking devbox above a bare flake by
claiming a root `flake.nix` in a devbox project is usually generated
from `devbox.json`. That is false: devbox writes its generated flake
under `.devbox/gen/flake/`, never to the project root, so a root flake
sitting beside a `devbox.json` was authored deliberately. The ordering
survives; the reasoning behind it does not, and is not restated as if it
did.)

#### Scenario: Init on a flox project
- **WHEN** `.flox/` exists in the project root
- **THEN** the generated manifest sets `provider = "flox"` and the sandbox
  name defaults to the directory slug

#### Scenario: Init on a devbox project
- **WHEN** `devbox.json` exists in the project root and no `.flox/` does
- **THEN** the generated manifest sets `provider = "devbox"`
- **AND** if no `devbox.lock` exists, init prints `devbox install` as the
  next step before `up` — devbox has no separate lock subcommand, so
  naming one would send the user to a command that does not exist

#### Scenario: Init on a devbox project declaring no packages
- **WHEN** `devbox.json` exists, declares no packages, and no
  `devbox.lock` exists
- **THEN** init still prints `devbox install` as the next step, matching
  the `env-provider` rule that a zero-package project has something to
  resolve after all: devbox's own base nixpkgs entry, which is the
  floating `nixpkgs-unstable` branch until a lockfile pins it

#### Scenario: Init on a fully locked devbox project
- **WHEN** `devbox.json` and `devbox.lock` both exist
- **THEN** init reports the project as ready for `up` rather than
  advising a lock step

#### Scenario: Init on a nix flake project
- **WHEN** `flake.nix` exists in the project root and neither `.flox/`
  nor `devbox.json` does
- **THEN** the generated manifest sets `provider = "nix"`
- **AND** if `flake.lock` is absent, init prints the next step: run
  `nix flake lock` before `up`

#### Scenario: devbox and a flake both present
- **WHEN** `devbox.json` and `flake.nix` both exist and no `.flox/` does
- **THEN** the generated manifest sets `provider = "devbox"`
- **AND** init states in one line that a flake was also found and that
  `provider = "nix"` is available

#### Scenario: Both flox and a flake present
- **WHEN** `.flox/` and `flake.nix` both exist in the project root
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init states in one line that a flake was also found and that
  `provider = "nix"` is available

#### Scenario: Init on a Rust project with a pinned toolchain
- **WHEN** `rust-toolchain.toml` exists and none of `.flox/`,
  `devbox.json`, or `flake.nix` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init offers to generate a flox manifest honoring the pinned
  channel (via fenix or rust-overlay) together with a C toolchain,
  explaining in one line that rustup alone cannot provide a complete
  build environment

#### Scenario: Init on a Node project with a pinned version
- **WHEN** `.nvmrc` exists and none of `.flox/`, `devbox.json`, or
  `flake.nix` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init explains in one line that nvm alone cannot provide a
  complete build environment, and to run `flox init` and pin the Node.js
  version from `.nvmrc` before `up`

#### Scenario: Init on a Python project with a pinned version
- **WHEN** `.python-version` exists and none of `.flox/`, `devbox.json`,
  or `flake.nix` does
- **THEN** the generated manifest sets `provider = "flox"`
- **AND** init explains in one line that pyenv alone cannot provide a
  complete build environment, and to run `flox init` and pin the Python
  version from `.python-version` before `up`

#### Scenario: An existing environment supersedes any toolchain pin
- **WHEN** `.flox/`, `devbox.json`, or `flake.nix` exists alongside one
  or more of `rust-toolchain.toml`, `.nvmrc`, `.python-version`
- **THEN** init reports the project as ready for `up` (or, for an
  unlocked environment, one lock command away from it) and does not
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
- **WHEN** none of `.flox/`, `devbox.json`, or `flake.nix` exists in the
  project root
- **THEN** the manifest is still generated with `provider = "flox"`
- **AND** init prints the next step: run `flox init` before `up`

### Requirement: doctor
The system SHALL provide `devcroft doctor` reporting, per requirement:
whether the running host can enforce the process tier at all, the
provider the discovered manifest declares, ssh config managed-section
state, and which manifest aspects would be degraded on this host. Output
SHALL be actionable — each failure names the fix.

The backend check SHALL report a **probed platform capability**, not the
presence or version of an external binary. The process tier links its
enforcement backend as a library, so there is no binary to look for and
no version to range-check; what remains is whether this kernel actually
supports the mechanism, which SHALL be determined by attempting it
rather than inferred from a version string or a binary on `PATH`.

The provider check SHALL report **the provider the project declares, and
no others**. A provider devcroft supports but this project does not use
SHALL NOT cause a failure, and SHALL NOT be reported as a problem —
a project's environment is not broken because a different provider is
absent. Where no manifest is discoverable, `doctor` SHALL report on the
providers it finds without requiring any of them, since it cannot know
what a future manifest will declare.

#### Scenario: Backend capability probed, not inferred
- **WHEN** `doctor` runs on a host whose kernel does not support the
  process tier's enforcement mechanism
- **THEN** `doctor` reports FAIL for the backend, naming the platform and
  what was missing, even though no external binary is involved

#### Scenario: Only the declared provider is required
- **WHEN** the discovered manifest declares one provider, and a different
  supported provider's tooling is absent from the host
- **THEN** `doctor` reports on the declared provider only, does not
  mention the absent one, and does not fail on its account

#### Scenario: The declared provider is missing
- **WHEN** the discovered manifest declares a provider whose tooling is
  not usable on this host
- **THEN** `doctor` reports FAIL for that provider, naming it and how to
  install it

#### Scenario: nix present but flakes disabled
- **WHEN** the project declares `provider = "nix"`, `nix` is on PATH, but
  flake commands are rejected (experimental features disabled)
- **THEN** `doctor` reports FAIL for the nix provider and names the
  configuration change that enables flakes

#### Scenario: devbox present but Nix is not
- **WHEN** the project declares `provider = "devbox"`, `devbox` is on
  PATH, but Nix is not usable
- **THEN** `doctor` reports FAIL naming Nix as devbox's own unmet
  requirement, rather than suggesting a different provider

#### Scenario: No manifest to scope by
- **WHEN** `doctor` runs where no manifest is discoverable
- **THEN** it reports what it finds for each supported provider and
  requires none of them, so their absence does not fail the run

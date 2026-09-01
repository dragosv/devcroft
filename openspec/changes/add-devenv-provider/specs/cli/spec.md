# cli Delta Specification (add-devenv-provider)

## MODIFIED Requirements

### Requirement: init
The system SHALL provide `devcroft init` which detects an existing flox
environment (`.flox/`), an existing devbox project (`devbox.json`), an
existing devenv project (`devenv.nix`), an existing nix flake
(`flake.nix`), single-ecosystem toolchain pins (`rust-toolchain.toml`,
`.nvmrc`, `.python-version`), and the project language, generates a
minimal `devcroft.toml` with commented-out common options, and never
overwrites an existing manifest without `--force`.

Where several environments are present, detection SHALL apply a fixed,
documented order — flox, then devbox, then devenv, then a bare flake —
and SHALL state in one line which others were found and remain available.

The order is a deterministic tiebreak, not a judgement that the losers
are derived artifacts. Each is a genuine, hand-maintained environment
definition, and a project carrying two of them has made a choice devcroft
cannot infer. What matters for correctness is that the result is
predictable and that the alternatives are named, so a user who wanted the
other one can see it and set `env.provider` themselves.

A bare flake ranks last for the same reason it already did: it is the
most generic signal of the four, and the three named tools each write
their generated flakes elsewhere than the project root. Measured for
devenv 2.2.2: it generates no root `flake.nix` at all, keeping its
evaluation artifacts under `.devenv/`, so a root flake beside a
`devenv.nix` was authored deliberately.

#### Scenario: Init on a flox project
- **WHEN** `.flox/` exists in the project root
- **THEN** the generated manifest sets `provider = "flox"` and the sandbox
  name defaults to the directory slug

#### Scenario: Init on a devbox project
- **WHEN** `devbox.json` exists in the project root and no `.flox/` does
- **THEN** the generated manifest sets `provider = "devbox"`
- **AND** if no `devbox.lock` exists, init prints `devbox install` as the
  next step before `up`

#### Scenario: Init on a devenv project
- **WHEN** `devenv.nix` exists in the project root and neither `.flox/`
  nor `devbox.json` does
- **THEN** the generated manifest sets `provider = "devenv"`
- **AND** if no `devenv.lock` exists, init prints `devenv update` as the
  next step before `up`

#### Scenario: Init on a fully locked devenv project
- **WHEN** `devenv.nix` and `devenv.lock` both exist
- **THEN** init reports the project as ready for `up` rather than advising
  a lock step

#### Scenario: devenv and a flake both present
- **WHEN** `devenv.nix` and `flake.nix` both exist and neither `.flox/`
  nor `devbox.json` does
- **THEN** the generated manifest sets `provider = "devenv"`
- **AND** init states in one line that a flake was also found and that
  `provider = "nix"` is available

#### Scenario: Init on a nix flake project
- **WHEN** `flake.nix` exists in the project root and none of `.flox/`,
  `devbox.json` or `devenv.nix` does
- **THEN** the generated manifest sets `provider = "nix"`
- **AND** if `flake.lock` is absent, init prints the next step: run
  `nix flake lock` before `up`

#### Scenario: An existing environment supersedes any toolchain pin
- **WHEN** `.flox/`, `devbox.json`, `devenv.nix` or `flake.nix` exists
  alongside one or more of `rust-toolchain.toml`, `.nvmrc`,
  `.python-version`
- **THEN** init reports the project as ready for `up` (or, for an unlocked
  environment, one lock command away from it) and does not print
  toolchain-pin advice

#### Scenario: Init without an environment
- **WHEN** none of `.flox/`, `devbox.json`, `devenv.nix` or `flake.nix`
  exists in the project root
- **THEN** the manifest is still generated with `provider = "flox"`
- **AND** init prints the next step: run `flox init` before `up`

### Requirement: doctor
The system SHALL provide `devcroft doctor` reporting, per requirement:
whether the running host can enforce the process tier at all, the
provider the discovered manifest declares, ssh config managed-section
state, and which manifest aspects would be degraded on this host. Output
SHALL be actionable — each failure names the fix.

The backend check SHALL report a **probed platform capability**, not the
presence or version of an external binary.

The provider check SHALL report **the provider the project declares, and
no others**. A provider devcroft supports but this project does not use
SHALL NOT cause a failure, and SHALL NOT be reported as a problem. Where
no manifest is discoverable, `doctor` SHALL report on the providers it
finds without requiring any of them.

Where a provider is a frontend over another tool, an unmet requirement
SHALL be reported as **that provider's own** unmet requirement, not as a
suggestion to switch providers.

#### Scenario: Backend capability probed, not inferred
- **WHEN** `doctor` runs on a host whose kernel does not support the
  process tier's enforcement mechanism
- **THEN** `doctor` reports FAIL for the backend, naming the platform and
  what was missing

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

#### Scenario: devenv present but Nix is not
- **WHEN** the project declares `provider = "devenv"`, `devenv` is on
  PATH, but Nix is not usable
- **THEN** `doctor` reports FAIL naming Nix as devenv's own unmet
  requirement, rather than suggesting a different provider

#### Scenario: No manifest to scope by
- **WHEN** `doctor` runs where no manifest is discoverable
- **THEN** it reports what it finds for each supported provider and
  requires none of them

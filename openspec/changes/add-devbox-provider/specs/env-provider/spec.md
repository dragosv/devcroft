# env-provider Delta Specification (add-devbox-provider)

## ADDED Requirements

### Requirement: devbox provider resolution
The system SHALL resolve a devbox environment by running devbox's own
activation once at `up`, host-side and before any restriction applies,
capturing the result as a diff against the same fixed pre-activation
baseline the flox and nix providers use (a real `HOME`, a conventional
`PATH`, nothing else), and injecting the diff into the keeper.

Resolution SHALL respect the project's lockfile and SHALL NOT update it,
resolve package versions, or contact a package index to decide *what* to
install. Materializing already-pinned packages is permitted and expected;
choosing versions at `up` is not.

Resolution SHALL require `devbox.json` in the project root before any
devbox command runs, failing at layer `provider` with exit code 3 and a
hint to initialize a devbox project.

The lockfile precondition SHALL be expressed as **"nothing resolves at
`up`"**, not as "a lockfile exists". Specifically, every package the
project declares SHALL have a recorded resolution covering **the system
`up` is running on**; where any declared package does not, `up` SHALL
fail at layer `provider` with exit code 3, naming the package and the
command that records it.

Two properties of devbox's lockfile make the weaker "file exists" check
insufficient, both observed against devbox 0.18.0:

- A project that declares no packages has **no lockfile at all**, and is
  a legitimate — if minimal — environment. Requiring the file would
  reject it for a reason that is not true of it.
- Resolutions are recorded **per system**, and the recorded set is not
  guaranteed to include the current one. A lockfile committed from
  another platform can be present and complete for that platform while
  leaving this one unresolved, which a presence check accepts and which
  then resolves at `up` — the exact thing the precondition exists to
  prevent.

devbox is a frontend over Nix and cannot materialize anything without it.
The system SHALL therefore verify Nix is usable as part of the devbox
provider's own preconditions, and SHALL report a missing Nix as devbox's
unmet requirement rather than instructing the user to change providers.

#### Scenario: Toolchain from the devbox environment is visible in a session
- **WHEN** provider is `devbox` and `devbox.json` declares a package
  providing a compiler
- **AND** the sandbox is up
- **THEN** running that compiler through `devcroft exec` succeeds, without
  the profile granting write access to store or devbox internals

#### Scenario: Store paths become readable
- **WHEN** provider is `devbox`
- **THEN** the compiled policy includes read access to the resolved
  closure's store root, with every such rule carrying origin
  `provider:devbox`

#### Scenario: Toolchain works under network deny-all
- **WHEN** provider is `devbox` and `network.default = "deny"` with an
  empty allowlist
- **AND** the sandbox is up
- **THEN** every tool from the environment runs, because materialization
  happened at `up` on the host; no session-time network is required for
  the toolchain

#### Scenario: Activation is independent of the invoking shell
- **WHEN** the shell running `up` has extra directories on `PATH` or
  arbitrary extra environment variables set
- **THEN** none of them appear in the captured activation diff, and the
  diff is byte-identical to one captured from a clean shell

#### Scenario: Declared but unlocked package fails rather than resolving
- **WHEN** provider is `devbox` and `devbox.json` declares a package
  with no recorded resolution
- **THEN** `up` fails at layer `provider` with exit code 3, naming the
  package and the command that records it, rather than resolving it
  against whatever the package set currently points at

#### Scenario: Locked for another system only
- **WHEN** every declared package has a recorded resolution, but none
  covers the system `up` is running on
- **THEN** `up` fails the same way — a lockfile complete for a different
  platform is not a lockfile for this one

#### Scenario: A project declaring no packages needs no lockfile
- **WHEN** provider is `devbox`, `devbox.json` declares no packages, and
  no `devbox.lock` exists
- **THEN** `up` succeeds — there is nothing to resolve, so the
  precondition is satisfied rather than violated

#### Scenario: Missing environment, not missing feature
- **WHEN** provider is `devbox` and the project has no `devbox.json`
- **THEN** `up` fails at layer `provider` with exit code 3 and a hint to
  initialize a devbox project — the same "missing environment" answer
  flox and nix give, never a suggestion to use a degraded mode

#### Scenario: Nix unavailable
- **WHEN** provider is `devbox`, the project is fully locked, but Nix is
  not usable on this host
- **THEN** `up` fails at layer `provider` with exit code 3, naming Nix as
  a devbox requirement and how to install it

#### Scenario: Stale environment after devbox file change
- **WHEN** `devbox.json` or `devbox.lock` changed after the last `up`
- **THEN** `status` reports the environment as stale
- **AND** `up` prints a one-line notice suggesting `--recreate`

#### Scenario: A lockfile appearing is itself a change
- **WHEN** the last `up` ran against a project with no `devbox.lock`,
  and one now exists
- **THEN** the environment reports as stale — fingerprinting SHALL treat
  an absent lockfile as a distinct state, not as equivalent to an empty
  or unchanged one

### Requirement: devbox captures its environment without running the init hook
The devbox provider SHALL satisfy the general "Provisioning does not
execute project code" requirement (`fix-provisioning-hooks`) by using
the entry point that hands back an environment rather than the one that
runs a command inside it.

Measured against devbox 0.18.0: `devbox run` executes `shell.init_hook`,
`--pure` included, and `devbox shellenv` does not — under any variant,
including `--init-hook`, which only appends a source line to the emitted
text rather than executing anything.

This is not a marginal case. `devbox init` writes an `init_hook` into
every new `devbox.json`, so having one is the out-of-the-box state and
there is no population of hook-free devbox projects to fall back on.
The provider SHALL therefore report `false` for having run an activation
hook, and a test SHALL assert the hook does not run — the property is
devcroft's choice of entry point, not devbox's caution, and a later
switch to `devbox run` would silently reintroduce the violation.

#### Scenario: A project-defined init hook does not run at up
- **WHEN** provider is `devbox` and the project defines an
  initialization hook that would write a file or contact the network
- **THEN** resolution completes without that hook having run, observable
  by the file not existing and no request having been made
- **AND** `up` prints no activation-hook warning, because none ran

### Requirement: Resolution depends only on committed files
The system SHALL capture an environment that is a function of the
project's committed environment definition, not of machine-global state
belonging to the provider. Where a provider maintains a global or default
package set outside the project, the captured environment SHALL NOT
include it, or the provider SHALL fail at layer `provider` rather than
silently producing an environment another machine cannot reproduce.

#### Scenario: Global packages do not leak into the sandbox
- **WHEN** the host has provider-global packages installed that the
  project's own environment definition does not declare
- **THEN** those packages are absent from the captured environment, and
  the sandbox sees only what the project declares

## MODIFIED Requirements

### Requirement: Only declarative providers
The system SHALL reject any provider value that does not name a supported
declarative environment provider. Supported values are `flox`, `nix`
(with `flake` and `flakes` accepted as aliases normalized to `nix`), and
`devbox`. The rejection message SHALL distinguish "not yet supported"
(mise, devenv, pixi, hermit — qualified but unscheduled) from "out of
scope by design" (`host`, `none` — devcroft has no non-reproducible
mode) and from "fails the qualification test" (version managers).

#### Scenario: Passthrough rejected
- **WHEN** the manifest declares `provider = "host"`
- **THEN** validation fails with exit code 2, layer `config`, and a message
  that devcroft has no non-reproducible mode

#### Scenario: Planned provider rejected
- **WHEN** the manifest declares `provider = "mise"`
- **THEN** validation fails with exit code 2 and a message that mise support
  is planned (artifact tier) but not yet implemented

#### Scenario: nix accepted
- **WHEN** the manifest declares `provider = "nix"`, `provider = "flake"`,
  or `provider = "flakes"`
- **THEN** validation succeeds and the provider resolves as `nix`
  everywhere the provider name is shown (`status`, policy rule origins)

#### Scenario: devbox accepted
- **WHEN** the manifest declares `provider = "devbox"`
- **THEN** validation succeeds and the provider resolves as `devbox`
  everywhere the provider name is shown (`status`, policy rule origins)

#### Scenario: Missing environment, not missing feature
- **WHEN** provider is `flox` (explicit or default) and the project has no
  `.flox/` environment
- **THEN** `up` fails at layer `provider` with exit code 3 and the hint
  `flox init`

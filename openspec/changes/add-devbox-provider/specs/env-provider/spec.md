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
project declares SHALL have **a key in `devbox.lock`'s `packages` map**;
where any declared package does not, `up` SHALL fail at layer `provider`
with exit code 3, naming the package and the command that records it.

A lockfile can be present and non-empty while omitting a specific
declared package entirely, e.g. after editing `devbox.json` by hand
without re-running `devbox install`. A presence check on the *file*
accepts this; a presence check on the *key* does not.

**That per-key check is necessary but not sufficient**, and the system
SHALL additionally verify, *after* capture, that `devbox.lock` is
byte-identical to what it was before. Where it is not, the system SHALL
restore the original file — or delete one capture created where the
project had none — and fail at layer `provider` with exit code 3, so a
refused `up` leaves the working tree exactly as it found it rather than
reporting a violation it has already committed.

The post-check is required because the per-key check structurally cannot
see everything devbox resolves: `devbox.lock` also carries devbox's own
**base nixpkgs entry**, which is not a declared package. Measured against
devbox 0.18.0 — a project whose every declared package was fully
resolved, but whose lockfile carried no `github:NixOS/nixpkgs/…` entry,
passed every precondition, and `up` then resolved that entry live against
the floating `nixpkgs-unstable` branch and wrote it to disk.

The check SHALL be a byte comparison rather than a prediction of which
keys devbox needs. The base entry's key is not a constant: measured, a
project pinning `nixpkgs.commit` in `devbox.json` locks under
`github:NixOS/nixpkgs/<that commit>` instead. Predicting the full key set
would mean reimplementing devbox's resolution rules, which design.md
decision 1 rejects on drift grounds.

**Corrected by measurement, superseding an earlier draft of this
requirement.** The draft additionally required the package's lock entry
to cover **the system `up` is running on** specifically, reasoning that a
lockfile resolved only for another platform leaves the current one
unresolved. Measured against devbox 0.18.0 with the exact capture command
decision 1 chose (`sh -c 'eval "$(devbox shellenv --pure)"; env -0'`),
that is false: a package entry present for a *different* system only
(e.g. `x86_64-darwin`, host running `aarch64-linux`) resolves and
materializes cleanly, without touching the lockfile on disk. devbox
resolves any system from the entry's pinned `resolved` commit reference,
which is system-independent — only the *output path* varies by system,
and the commit is already fixed. A `.lock` entry existing at all is what
makes resolution reproducible; which systems happen to be cached under it
is not load-bearing.

What the same measurement confirms **does** violate the two-phase rule:
a declared package with **no key in `devbox.lock` at all** causes the
capture command to resolve it live against `nixpkgs-unstable` — a
floating reference, not a pinned commit — and **write the new resolution
back into `devbox.lock` on disk**. That is "resolve package versions" and
"update it", which the paragraph above forbids.

**One clause of an earlier version of this paragraph was false and is
withdrawn**, since it was doing real work in the argument: it also cited
"contacting `cache.nixos.org`" as evidence of the violation. A cold-store
measurement (a package never before materialized on the host, locked only
for a foreign system) shows the *permitted* case fetches from
`cache.nixos.org` too — 13 MiB of it — while leaving the lockfile
untouched. Fetching a pinned store path is exactly the "materializing
already-pinned packages is permitted and expected" case. So the binary
cache fetch does not distinguish the two situations at all, and the
discriminator is solely whether the lockfile changes.

The system SHALL read `devbox.json` by the same rules devbox itself
applies, and SHALL NOT reject a file devbox accepts. Measured against
devbox 0.18.0 by feeding it each relaxation in turn: line comments
(`//`), block comments (`/* */`), and trailing commas are accepted; hash
comments, single-quoted strings, and unquoted keys are not. Comments and
trailing commas inside string *values* are data and SHALL be preserved.

#### Scenario: A commented devbox.json is read, not rejected
- **WHEN** `devbox.json` contains line comments, block comments, or
  trailing commas — all of which devbox itself accepts, and which its own
  documentation shows
- **THEN** `up` reads it successfully rather than failing at layer
  `provider` with a parse error

#### Scenario: Comment syntax inside a string stays data
- **WHEN** a declared package or other string value contains `//`, `/*`,
  or a comma
- **THEN** the value is preserved exactly, not truncated or split

### Requirement: devbox requires a usable Nix
devbox is a frontend over Nix and cannot materialize anything without it.
The system SHALL therefore verify Nix is usable as part of the devbox
provider's own preconditions, and SHALL report a missing Nix as devbox's
unmet requirement rather than instructing the user to change providers.

Verification SHALL **probe the capability rather than infer it from the
binary being present**, matching the rule the `cli` spec already states
for `doctor` — a `nix` on `PATH` that cannot evaluate is the failure this
precondition exists to catch, and a presence check reports it as success.

#### Scenario: Nix present but unusable
- **WHEN** provider is `devbox`, `devbox` and `nix` are both on `PATH`,
  but `nix` cannot evaluate (experimental features disabled, unreachable
  daemon)
- **THEN** `up` fails at layer `provider` with exit code 3, naming Nix as
  devbox's own unmet requirement

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

#### Scenario: Locked for another system only succeeds
- **WHEN** a declared package's `devbox.lock` entry covers only a system
  other than the one `up` is running on
- **THEN** `up` succeeds — the entry's pinned commit reference resolves
  and materializes for the current system without writing back to
  `devbox.lock`, and per-system cache coverage is not part of this
  precondition (measured against devbox 0.18.0, cold store; see
  design.md)

#### Scenario: A project declaring no packages still needs a lockfile
- **WHEN** provider is `devbox`, `devbox.json` declares no packages, and
  no `devbox.lock` exists
- **THEN** `up` fails at layer `provider` with exit code 3, hinting
  `devbox install`, and no `devbox.lock` is left behind — devbox's stdenv
  comes from its base nixpkgs, which is the floating `nixpkgs-unstable`
  branch until a lockfile pins it, so "declares no packages" does not
  mean "has nothing to resolve"

#### Scenario: A complete lockfile survives provisioning untouched
- **WHEN** provider is `devbox` and `devbox.lock` covers everything
  devbox needs, as `devbox install` produces
- **THEN** `up` succeeds and `devbox.lock` is byte-identical afterwards

#### Scenario: Provisioning refuses rather than rewriting the lockfile
- **WHEN** capture would add or change any entry in `devbox.lock` —
  including devbox's own base nixpkgs entry, which no per-package
  precondition can see
- **THEN** `up` fails at layer `provider` with exit code 3, naming
  `devbox install` as the fix, and `devbox.lock` is left exactly as it
  was before `up` ran

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

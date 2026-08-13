# env-provider Specification

## Purpose

Resolve the development environment (toolchain, PATH, env vars) declared by
an environment provider, and make it available inside the sandbox.
Reproducibility is a defining property of devcroft: every sandbox is backed
by a declarative environment definition, and no passthrough provider
exists. MVP provider: `flox`.

## ADDED Requirements

### Requirement: Fixed composition order
The system SHALL resolve the environment BEFORE sandbox restrictions are
applied, and SHALL apply sandbox restrictions to the process tree that
already carries the resolved environment. Provider activation MUST NOT run
with wider privileges than the supervisor itself.

#### Scenario: Environment visible in session
- **WHEN** provider is `flox` and the flox env provides `zig`
- **AND** the sandbox is up
- **THEN** `devcroft exec -- zig version` succeeds without the profile
  granting write access to flox internals

### Requirement: flox provider resolution
The system SHALL resolve a flox environment by running its activation once
at `up`, capturing the resulting environment as a diff against the
pre-activation environment, and injecting the diff into the keeper. Both
activation and the pre-activation baseline it is diffed against SHALL run
in a fixed environment (a real `HOME`, and nothing else beyond a
conventional `PATH`) rather than whatever environment invoked `up`, so the
same manifest resolves the same diff regardless of the operator's own
shell or locally-installed tools.

#### Scenario: Store paths become readable
- **WHEN** provider is `flox`
- **THEN** the compiled policy automatically includes read access to
  `/nix/store` (or the flox store root) without the user declaring it

#### Scenario: Toolchain works under network deny-all
- **WHEN** `network.default = "deny"` with an empty allowlist
- **AND** the sandbox is up
- **THEN** every tool from the environment runs, because materialization
  happened at `up` on the host; no session-time network is required for
  the toolchain

#### Scenario: Activation is independent of the invoking shell
- **WHEN** the shell running `up` has extra directories on `PATH` (a
  personal `~/bin`, `nvm`, `rustup`, ...) or arbitrary extra environment
  variables set
- **THEN** none of them appear in the captured activation diff — the
  diff reflects only what the manifest's own activation changed

#### Scenario: Missing flox binary
- **WHEN** provider is `flox` and `flox` is not on PATH
- **THEN** `up` fails with layer `provider`, exit code 3, and a hint to run
  `devcroft doctor`

#### Scenario: Stale environment after manifest change
- **WHEN** the flox `manifest.toml` changed after the last `up`
- **THEN** `status` reports the environment as stale
- **AND** `up` prints a one-line notice suggesting `--recreate`

### Requirement: Only declarative providers
The system SHALL reject any provider value that does not name a supported
declarative environment provider. The rejection message SHALL distinguish
"not yet supported" (nix flakes, devbox, mise — planned in that order,
see deferred changes)
from "out of scope by design" (`host`, `none` — devcroft has no
non-reproducible mode).

#### Scenario: Passthrough rejected
- **WHEN** the manifest declares `provider = "host"`
- **THEN** validation fails with exit code 2, layer `config`, and a message
  that devcroft has no non-reproducible mode

#### Scenario: Planned provider rejected in MVP
- **WHEN** the manifest declares `provider = "mise"`
- **THEN** validation fails with exit code 2 and a message that mise support
  is planned (artifact tier) but not yet implemented

#### Scenario: Missing environment, not missing feature
- **WHEN** provider is `flox` (explicit or default) and the project has no
  `.flox/` environment
- **THEN** `up` fails at layer `provider` with exit code 3 and the hint
  `flox init`

### Requirement: Provider does not weaken the sandbox
The system SHALL NOT let provider resolution add filesystem or network
grants beyond the provider's documented read-only store paths.

#### Scenario: Activation script attempts a write grant
- **WHEN** a provider activation would require write access outside the
  project root to function
- **THEN** `up` fails with layer `provider` and names the offending path,
  rather than silently widening the policy

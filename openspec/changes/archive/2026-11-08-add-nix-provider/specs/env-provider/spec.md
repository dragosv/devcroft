# env-provider Delta Specification (add-nix-provider)

## ADDED Requirements

### Requirement: nix provider resolution
The system SHALL resolve a nix flakes environment by running the flake's
default dev shell activation once at `up`, capturing the resulting
environment as a diff against the same fixed pre-activation baseline the
flox provider uses (a real `HOME`, a conventional `PATH`, nothing else),
and injecting the diff into the keeper. Resolution SHALL run with the
flake's lockfile respected and never updated (`--no-update-lock-file`
semantics) and SHALL NOT pass `--impure`. Resolution SHALL require
`flake.nix` and `flake.lock` to exist in the project root before any nix
command runs, failing at layer `provider` with exit code 3 and hints
`nix flake init` and `nix flake lock` respectively.

#### Scenario: Toolchain from the dev shell is visible in a session
- **WHEN** provider is `nix` and the flake's default dev shell provides
  `zig`
- **AND** the sandbox is up
- **THEN** `devcroft exec -- zig version` succeeds without the profile
  granting write access to nix internals

#### Scenario: Store paths become readable
- **WHEN** provider is `nix`
- **THEN** the compiled policy automatically includes read access to the
  nix store root, with every such rule carrying origin `provider:nix`

#### Scenario: Toolchain works under network deny-all
- **WHEN** provider is `nix` and `network.default = "deny"` with an empty
  allowlist
- **AND** the sandbox is up
- **THEN** every tool from the dev shell runs, because materialization
  happened at `up` on the host; no session-time network is required for
  the toolchain

#### Scenario: Activation is independent of the invoking shell
- **WHEN** the shell running `up` has extra directories on `PATH` or
  arbitrary extra environment variables set
- **THEN** none of them appear in the captured activation diff — the
  diff reflects only what the flake's dev shell activation changed

#### Scenario: Missing flake.lock
- **WHEN** provider is `nix`, `flake.nix` exists, and `flake.lock` does
  not
- **THEN** `up` fails with layer `provider`, exit code 3, and the hint
  `nix flake lock`, without resolving any flake input from the network

#### Scenario: Lockfile does not cover a flake input
- **WHEN** `flake.nix` declares an input that `flake.lock` has no entry
  for
- **THEN** `up` fails at layer `provider` rather than silently resolving
  the input and rewriting the lockfile

#### Scenario: Missing nix binary
- **WHEN** provider is `nix` and `nix` is not on PATH
- **THEN** `up` fails with layer `provider`, exit code 3, and a hint to
  run `devcroft doctor`

#### Scenario: Flakes not enabled
- **WHEN** provider is `nix` and the installed nix rejects flake commands
  (experimental features disabled)
- **THEN** `up` fails with layer `provider`, exit code 3, and a message
  naming the nix configuration change required

#### Scenario: Missing environment, not missing feature
- **WHEN** provider is `nix` and the project has no `flake.nix`
- **THEN** `up` fails at layer `provider` with exit code 3 and the hint
  `nix flake init`

#### Scenario: Stale environment after flake change
- **WHEN** `flake.nix` or `flake.lock` changed after the last `up`
- **THEN** `status` reports the environment as stale
- **AND** `up` prints a one-line notice suggesting `--recreate`

## MODIFIED Requirements

### Requirement: Only declarative providers
The system SHALL reject any provider value that does not name a supported
declarative environment provider. Supported values are `flox` and `nix`
(with `flake` and `flakes` accepted as aliases normalized to `nix`). The
rejection message SHALL distinguish "not yet supported" (devbox, mise —
planned in that order, see deferred changes) from "out of scope by
design" (`host`, `none` — devcroft has no non-reproducible mode).

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

#### Scenario: Missing environment, not missing feature
- **WHEN** provider is `flox` (explicit or default) and the project has no
  `.flox/` environment
- **THEN** `up` fails at layer `provider` with exit code 3 and the hint
  `flox init`

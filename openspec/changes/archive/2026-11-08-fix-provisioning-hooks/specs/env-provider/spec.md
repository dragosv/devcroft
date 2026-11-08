# env-provider Delta Specification (fix-provisioning-hooks)

## ADDED Requirements

### Requirement: Provisioning does not execute project code
The system SHALL NOT execute project-supplied commands during provider
resolution where the provider offers any way to capture its environment
without doing so, and SHALL prefer that way even when it is less
convenient than an alternative that executes them.

This is the two-phase execution rule applied to the provider itself.
Provisioning runs host-side, before any restriction exists, with the
invoking user's full network and filesystem access. The rule's stated
justification is that this phase runs pinned tooling from a lockfile
rather than project code; an activation hook defined in the project's
own environment file is project code, and running it voids that
justification rather than bending it.

Where a provider offers **no** such way, the system SHALL detect the
construct it cannot skip and SHALL report it at `up` — exactly one
warning naming the provider, the construct, and that it runs
unconfined. The system SHALL NOT fail `up` on this account: an
activation hook is an ordinary, widely-used feature of the providers
that have one, and the same code runs when the user activates the
environment by hand.

The system SHALL NOT report the warning for a project whose environment
defines no such construct.

#### Scenario: A capturable provider does not run the hook
- **WHEN** a project's environment defines an activation hook that would
  write a file, and the provider offers a hook-free capture
- **THEN** resolution completes and the file does not exist

#### Scenario: An unavoidable hook is reported once
- **WHEN** a project's environment defines an activation hook and the
  provider offers no way to capture without running it
- **THEN** `up` succeeds and prints exactly one warning naming the
  provider and the construct
- **AND** the warning is not repeated per session

#### Scenario: No hook, no warning
- **WHEN** a project's environment defines no activation hook
- **THEN** `up` prints no such warning, and the captured environment is
  unchanged from before this requirement existed

## MODIFIED Requirements

### Requirement: nix provider resolution
The system SHALL resolve a nix flakes environment by reading the flake's
default dev shell build environment once at `up`, **as structured data
rather than by entering the shell**, capturing it as a diff against the
same fixed pre-activation baseline the flox provider uses (a real
`HOME`, a conventional `PATH`, nothing else), and injecting the diff
into the keeper.

The mechanism SHALL NOT execute the dev shell's `shellHook`. Entering
the dev shell to dump its environment does run it, and so does
evaluating the shell script form of the build environment — that script
ends by evaluating the hook itself. Reading the build environment in its
structured form carries the hook through as inert data, which is what
this requirement demands.

Resolution SHALL run with the flake's lockfile respected and never
updated (`--no-update-lock-file` semantics) and SHALL NOT pass
`--impure`. Resolution SHALL require `flake.nix` and `flake.lock` to
exist in the project root before any nix command runs, failing at layer
`provider` with exit code 3 and hints `nix flake init` and
`nix flake lock` respectively.

#### Scenario: The dev shell's hook does not run
- **WHEN** provider is `nix` and the flake's dev shell defines a
  `shellHook` that writes a file
- **AND** `up` resolves the environment
- **THEN** the file does not exist, and the captured environment is
  otherwise the same as a capture of the same flake without the hook

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

# env-provider Delta Specification (add-devenv-provider)

## ADDED Requirements

### Requirement: devenv provider resolution
The system SHALL resolve a devenv environment once at `up`, host-side and
before any restriction applies, capturing the result as a diff against
the same fixed pre-activation baseline the flox, nix and devbox providers
use (a real `HOME`, a conventional `PATH`, nothing else), and injecting
the diff into the keeper.

Resolution SHALL respect `devenv.lock` and SHALL NOT update it, resolve
input revisions, or contact a package index to decide *what* to
materialize. Materializing already-pinned inputs is permitted and
expected; choosing revisions at `up` is not.

Resolution SHALL require `devenv.nix` in the project root before any
devenv command runs, failing at layer `provider` with exit code 3 and a
hint to initialize a devenv project.

The lockfile precondition SHALL be expressed as **"nothing resolves at
`up`"**, enforced by verifying, after capture, that `devenv.lock` is
byte-identical to what it was before. Where it is not, the system SHALL
restore the original file — or delete one that capture created where the
project had none — and fail at layer `provider` with exit code 3, so a
refused `up` leaves the working tree exactly as it found it.

A byte comparison is required rather than a prediction of which keys
devenv needs, for the reason `add-devbox-provider` recorded: predicting
the key set means reimplementing the provider's resolution rules, which
drifts silently when they change.

#### Scenario: Environment captured host-side
- **WHEN** `up` runs in a project declaring `provider = "devenv"` with
  `devenv.nix`, `devenv.yaml` and `devenv.lock` present
- **THEN** the devenv environment is resolved once, host-side, and the
  captured diff is injected into the keeper
- **AND** sessions started later inherit it without re-resolving

#### Scenario: Missing environment, not missing feature
- **WHEN** provider is `devenv` and the project has no `devenv.nix`
- **THEN** `up` fails at layer `provider` with exit code 3 and a hint to
  run `devenv init`

#### Scenario: Capture must not resolve
- **WHEN** capture would change `devenv.lock`
- **THEN** `up` fails at layer `provider` with exit code 3, and
  `devenv.lock` is left byte-identical to its pre-`up` content

#### Scenario: Capture is independent of the invoking shell
- **WHEN** `up` is run twice from shells with different `PATH` and
  environment
- **THEN** the captured env diff is byte-identical between the two runs

### Requirement: devenv captures its environment without running enterShell
`enterShell` is project code. The system SHALL capture a devenv
environment through an entry point that does not execute it, and SHALL
NOT use an entry point that does.

Measured against devenv 2.2.2, `devenv shell -- <cmd>` and
`devenv direnv-export` both execute `enterShell`; `devenv build`,
`devenv eval` and `devenv info` do not. Of those, only the `devenv build`
family yields a usable environment. The requirement is stated as a
property of the captured result rather than as a command name, so it
survives devenv changing its CLI: what SHALL hold is that no
project-defined code runs during provisioning.

Where no hook-free entry point yielding a complete environment can be
found, the system SHALL fail at layer `provider` rather than fall back to
one that runs project code, and SHALL NOT report the fallback as success.

#### Scenario: The hook does not run during provisioning
- **WHEN** a project's `enterShell` has an observable side effect outside
  the project root, and `up` resolves that environment
- **THEN** the side effect has not occurred when `up` returns

#### Scenario: A hook-running entry point is not silently substituted
- **WHEN** the hook-free entry point is unavailable or fails
- **THEN** `up` fails at layer `provider` naming the reason, rather than
  capturing through an entry point that runs `enterShell`

### Requirement: devenv's enterShell runs inside the sandbox
Where a project defines `enterShell`, the system SHALL capture it as data
during resolution and run it **inside** the sandbox, after restriction,
through the same mechanism that carries flox's `[hook].on-activate`.

The system SHALL report `ran_activation_hook` as false for devenv,
because nothing project-defined executes host-side. A devenv project with
an `enterShell` SHALL NOT produce the host-side-hook warning that a flox
project with `on-activate` produces, since the condition that warning
describes does not hold.

Consequence, stated because it is a real behaviour difference rather than
an implementation detail: an `enterShell` reaching for host tooling is
denied inside the sandbox, the same way a flox hook is.

#### Scenario: Hook runs once, inside
- **WHEN** a devenv project defines `enterShell` and `up` completes
- **THEN** the hook has executed exactly once, inside the sandbox, under
  the manifest's own compiled policy

#### Scenario: No host-side hook warning
- **WHEN** a devenv project defines `enterShell`
- **THEN** `up` does not warn that a project-defined hook ran host-side,
  because none did

#### Scenario: A hook needing host tooling is denied, not silently granted
- **WHEN** `enterShell` invokes a binary the compiled policy denies
- **THEN** the hook fails and `up` fails at layer `keeper`, naming the
  hook, rather than the sandbox coming up as though it had succeeded

### Requirement: devenv staleness covers all three declaration files
The system SHALL treat a devenv environment as stale when any of
`devenv.nix`, `devenv.yaml` or `devenv.lock` changes after `up`.

`devenv.yaml` is included because it carries the environment's inputs: it
can change what resolves without `devenv.nix` changing at all, so a
fingerprint over the other two would report a changed environment as
fresh.

#### Scenario: Editing any declaration file flips status
- **WHEN** any of `devenv.nix`, `devenv.yaml` or `devenv.lock` is edited
  after `up`
- **THEN** `status` reports the environment stale and `up` prints the
  `--recreate` notice

#### Scenario: Unrelated edits do not
- **WHEN** project files other than those three change
- **THEN** the environment is still reported fresh

## MODIFIED Requirements

### Requirement: Only declarative providers
The system SHALL reject any provider value that does not name a supported
declarative environment provider. Supported values are `flox`, `nix`
(with `flake` and `flakes` accepted as aliases normalized to `nix`),
`devbox`, and `devenv`. The rejection message SHALL distinguish "not yet
supported" (mise, pixi, hermit — qualified but unscheduled) from "out of
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

#### Scenario: devenv accepted
- **WHEN** the manifest declares `provider = "devenv"`
- **THEN** validation succeeds and the provider resolves as `devenv`
  everywhere the provider name is shown (`status`, policy rule origins)

#### Scenario: Missing environment, not missing feature
- **WHEN** provider is `flox` (explicit or default) and the project has no
  `.flox/` environment
- **THEN** `up` fails at layer `provider` with exit code 3 and the hint
  `flox init`

# env-provider

## ADDED Requirements

### Requirement: Provider resolution executes inside a sandbox

Provider activation SHALL run inside a provisioning sandbox. No provider's
activation SHALL execute unconfined on the host.

**Scoped to project shell, deliberately.** "Activation" here means the
arbitrary shell a project supplies and the provider executes — flox's
`[hook].on-activate`, devbox's `shell.init_hook`. It does **not** claim
that no repository-controlled input is interpreted host-side at all,
because for at least one supported provider that would be false:
resolving a nix project runs `nix flake metadata` and `nix print-dev-env
--json`, both of which evaluate `flake.nix`, which the repository
controls. Neither runs `shellHook` — that is the property
`fix-provisioning-hooks` measured and CLAUDE.md states — but evaluating a
repository's Nix expressions is not the same as executing nothing from
that repository.

The distinction is load-bearing rather than pedantic: a Nix evaluation is
constrained by the evaluator (a pure functional language, no arbitrary
syscalls, no ambient shell), while an activation hook is a shell with
whatever authority the calling process had. Confining the second is what
this change delivers. Narrowing the first — or qualifying which
evaluators are trusted to be sandboxes in their own right — is separate
work this requirement must not be read as having done.

#### Scenario: Activation with a hook

- **WHEN** a project's activation runs arbitrary shell as part of resolution
- **THEN** that shell executes inside the provisioning sandbox
- **AND** it is confined by the provisioning policy in force for that project

#### Scenario: Repository not reviewed by the user

- **WHEN** `up` runs on a repository whose contents the user has not inspected
- **THEN** no shell that repository supplies executes outside the provisioning
  sandbox
- **AND** the boundary applies before any of the project's own shell runs

#### Scenario: A provider evaluates repository-controlled definitions

- **WHEN** a provider resolves by evaluating a repository-controlled
  definition rather than by executing project shell — `flake.nix` under
  `nix print-dev-env --json`, a `devbox.json` under `shellenv --pure`
- **THEN** that evaluation is permitted to run host-side, bounded by the
  provider's own evaluator rather than by the provisioning sandbox
- **AND** the residual exposure is stated rather than implied: what
  constrains it is the evaluator's own semantics, and devcroft is
  relying on that rather than enforcing it

#### Scenario: Provider needs no execution to resolve

- **WHEN** a provider resolves by reading structured data, executing neither
  project shell nor a repository-controlled expression
- **THEN** it retains that path
- **AND** it is still described by a provisioning profile, so every provider's
  reach is inspectable by the same mechanism

### Requirement: The captured environment crosses the boundary as data

The resolved environment SHALL be transferred out of the provisioning sandbox as
data and parsed by the supervisor. It SHALL NOT be sourced or evaluated as
shell outside the sandbox.

#### Scenario: Environment is captured

- **WHEN** activation completes
- **THEN** the resulting environment is read from a descriptor the supervisor
  controls
- **AND** the existing baseline diff, store-grant derivation and staleness
  fingerprinting are applied to it unchanged

#### Scenario: Activation emits shell intended for evaluation

- **WHEN** the captured output contains constructs a caller would normally
  evaluate
- **THEN** they are parsed as data
- **AND** no part of the captured output executes on the host

### Requirement: Provisioning gets a substituted home directory

The provisioning sandbox SHALL provide a private home directory. Paths that must
persist across activations SHALL be declared and bound in explicitly.

#### Scenario: Hook writes to a package cache

- **WHEN** activation writes to a declared cache path
- **THEN** the write persists and is available to subsequent activations

#### Scenario: Hook writes to an undeclared home path

- **WHEN** activation writes to a path in the home directory that is not
  declared
- **THEN** the write does not reach the user's real home directory
- **AND** the difference from running the activation by hand is documented

#### Scenario: Sensitive host paths

- **WHEN** activation attempts to read credentials or keys outside the
  provisioning policy
- **THEN** the access is denied

### Requirement: Provisioning failures name their cause

When activation fails inside the provisioning sandbox, `up` SHALL report whether
the failure came from a policy denial and, where it did, name the path or
interface involved.

#### Scenario: Hook denied a path it needs

- **WHEN** activation fails because the provisioning policy denies a path
- **THEN** `up` fails at the provisioning layer and names that path
- **AND** the message distinguishes this from the hook itself being broken

#### Scenario: Hook fails on its own

- **WHEN** activation fails for a reason unrelated to policy
- **THEN** the provider's own error is reported without implying a denial

### Requirement: Provisioning confinement is claimed at its actual strength

Documentation and `up`'s output SHALL describe provisioning confinement as
applying the same tier of boundary as the rest of the sandbox. They SHALL NOT
imply that confined provisioning makes the tool suitable for code written to
escape.

#### Scenario: Operator reads what provisioning protects against

- **WHEN** the operator inspects what confined provisioning guarantees
- **THEN** it is stated as closing host-side execution, at the tier in force
- **AND** no wording suggests a stronger boundary than that tier provides

### Requirement: Provider resolution does not mutate the project's lockfile

Resolution SHALL leave the project's lockfile byte-identical, regardless of where
it executes.

#### Scenario: Activation would rewrite the lockfile

- **WHEN** activation writes to the lockfile during provisioning
- **THEN** the change is detected and the original restored
- **AND** `up` fails rather than proceeding from a mutated lockfile

### Requirement: Package-manager materialization authority is separate from activation code

The authority to materialize an environment — a `nix-daemon` socket or any
equivalent package-manager service — SHALL be modelled as its own capability,
separate from filesystem grants. Resolved stores SHALL remain read-only to both
provisioning and runtime. Project-controlled activation code SHALL NOT receive
that authority under any circumstances.

A provider that cannot demonstrably separate materialization from execution of
project-supplied activation code SHALL fail at layer `provider`, and SHALL NOT
be granted a writable store or a daemon connection as a fallback.

#### Scenario: Runtime uses a resolved Nix closure

- **WHEN** a sandbox runs against an environment resolved from a Nix closure
- **THEN** the store paths are granted read-only
- **AND** neither the runtime nor any activation code holds a daemon connection

#### Scenario: A provider that exposes no hook-free path

- **WHEN** a provider offers no documented way to materialize without running
  project activation code — flox, measured across every activation mode
- **THEN** devcroft constructs the separation itself, materializing from a
  derived environment it owns with the project's activation code removed
- **AND** the project's own environment definition is read, never rewritten
- **AND** the activation code then runs inside the provisioning sandbox,
  against the already-materialized environment

#### Scenario: Activation code that itself requires materialization authority

- **WHEN** a project's activation code needs to realise new packages while
  running, rather than declaring them as dependencies
- **THEN** it fails at layer `provider`, naming what it attempted
- **AND** no fallback grants it daemon authority or a writable store
- **AND** the error distinguishes this from "this provider cannot be confined",
  since the fix is to declare the dependency rather than to wait for anything

#### Scenario: Hook-free capture for providers that support it

- **WHEN** a provider offers a documented path that returns the environment
  without running project activation code (`nix print-dev-env --json`,
  `devbox shellenv --pure`)
- **THEN** that path is used, inside the provisioning worker
- **AND** the project's own activation script is never evaluated

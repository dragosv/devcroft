# env-provider

## ADDED Requirements

### Requirement: Provider resolution executes inside a sandbox

Provider activation SHALL run inside a provisioning sandbox. No provider's
activation SHALL execute unconfined on the host.

#### Scenario: Activation with a hook

- **WHEN** a project's activation runs arbitrary shell as part of resolution
- **THEN** that shell executes inside the provisioning sandbox
- **AND** it is confined by the provisioning policy in force for that project

#### Scenario: Repository not reviewed by the user

- **WHEN** `up` runs on a repository whose contents the user has not inspected
- **THEN** nothing in that repository executes outside the provisioning sandbox
- **AND** the boundary applies before any of the project's own code runs

#### Scenario: Provider needs no execution to resolve

- **WHEN** a provider resolves by reading structured data rather than executing
  project shell
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

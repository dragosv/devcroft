## ADDED Requirements

### Requirement: VM environment selection is explicit and deterministic

The VM path SHALL resolve explicit selection, external binding, project definition,
unique detection, then a named versioned base in that order. It SHALL report origin,
target and identity and SHALL NOT replace an invalid explicit selection by fallback.

#### Scenario: No environment is declared

- **WHEN** a project has no definition or detected provider
- **THEN** a named versioned base provides a private local environment
- **AND** status reports project setup undefined and fleet ineligible

#### Scenario: Several providers are detected without a terminal

- **WHEN** no explicit selection resolves the ambiguity
- **THEN** the command exits with candidates and the means to select one
- **AND** it executes no project code and does not wait for input

### Requirement: Local adaptation does not modify shared environments

The system SHALL permit policy-authorized dependency installation in private guest
state through an explicit setup window, retain it across restart and distinguish
it from a verified definition. Administrative setup SHALL NOT gain host or sibling
authority.

#### Scenario: A system library is installed interactively

- **WHEN** setup succeeds in one instance
- **THEN** that instance can use it after restart
- **AND** sibling environments and the shared base remain unchanged
- **AND** the installation alone does not make the instance fleet-eligible

### Requirement: A base declares how its system packages are pinned

A base SHALL declare whether a saved recipe can pin its system packages by content
hash. A base that cannot SHALL remain usable locally and SHALL be reported as
unable to reach strict fleet eligibility.

#### Scenario: A base installs from a mutable mirror

- **WHEN** a saved recipe depends on a package the base cannot pin by hash
- **THEN** status reports that the recipe cannot reach strict eligibility
- **AND** local use continues

### Requirement: Definitions can be saved externally without granting authority

An explicit save SHALL support repository-local and user-owned external definitions.
It SHALL record declared preparation inputs and omissions without claiming complete
inference. External bindings SHALL include project identity, root/subdirectory,
origin context, target and definition revision; grants SHALL remain separate.

#### Scenario: Another checkout reuses the directory basename

- **WHEN** it lacks the explicit project binding
- **THEN** it does not inherit the previous definition, grants or verification

#### Scenario: Saving a draft outside the repository

- **WHEN** the user selects an external destination
- **THEN** repository files remain unchanged and incomplete fields are identified
- **AND** secret values are not written into the definition

### Requirement: Dev Containers support is bounded by effective configuration

The image adapter SHALL resolve a platform digest, merge supported image metadata
and project fields, and validate the effective configuration before setup. Every
key of the pinned schema SHALL have a published class: honored, ignored with one
warning, or refused by field path. Refusal SHALL cover fields needing host
authority or an execution the profile does not perform, including inherited
metadata. Keys outside the pinned schema SHALL be ignored with one warning.

#### Scenario: An official image carries container-scoped metadata

- **WHEN** image metadata holds `capAdd`, `securityOpt` or baked Feature records
- **THEN** those fields are ignored with one warning naming them
- **AND** the image runs, since they confer no host authority

#### Scenario: The project requests forwarded ports

- **WHEN** the effective configuration contains `forwardPorts`
- **THEN** one warning names explicit SSH local forwarding as the way to reach them
- **AND** the environment is not refused for it

#### Scenario: An image has no variant for the guest architecture

- **WHEN** the digest resolves to no manifest for the Linux target
- **THEN** the image is refused by name, with no emulation

#### Scenario: Image metadata requests a privileged mount

- **WHEN** the project JSON itself contains only a supported image reference
- **THEN** effective configuration validation still refuses the mount
- **AND** no host mount or socket is supplied

#### Scenario: Repository requires Dockerfile or Features

- **WHEN** those fields occur in the effective definition
- **THEN** the adapter refuses them as outside the initial profile
- **AND** it does not invoke a host builder or silently omit them

### Requirement: Lifecycle phases execute only in the guest with durable results

Supported creation phases SHALL run in order once per successful creation, as
setup, and the start phase SHALL run on each start under runtime policy. Failure
SHALL identify the phase and prevent a false project-ready result. Retry of arbitrary partially executed lifecycle code
SHALL require an explicit operation.

#### Scenario: Post-create fails after writing files

- **WHEN** the instance is reopened
- **THEN** failure and partial state remain visible
- **AND** the phase is not silently marked complete or automatically rerun

### Requirement: Local status distinguishes availability from eligibility

Status SHALL separately expose VM/keeper health, workspace location, selected
environment, setup/service state and fleet eligibility. It SHALL NOT claim all
local changes are understood or a pinned image alone is reconstructible.

#### Scenario: Image boots but project dependencies are incomplete

- **WHEN** the user inspects status
- **THEN** the usable local shell and incomplete project preparation are both visible

# env-provider

## ADDED Requirements

### Requirement: Environment configuration resolves in a fixed order

Configuration SHALL resolve as: explicit flags, then a devcroft manifest, then a
detected provider signature file, then failure. The order SHALL be identical on
every command path.

#### Scenario: Manifest present

- **WHEN** a devcroft manifest is present and no overriding flag is given
- **THEN** the manifest determines the environment

#### Scenario: No manifest, provider file detected

- **WHEN** no manifest is present and a provider signature file is found
- **THEN** that provider is used
- **AND** the detected file and chosen provider are reported

#### Scenario: Explicit flag with a manifest present

- **WHEN** a provider is given explicitly on a project that has a manifest
- **THEN** the flag wins
- **AND** the override is reported

#### Scenario: Nothing found

- **WHEN** no manifest, no signature file, and no packages are supplied
- **THEN** the command fails, naming what was looked for and the flags that
  would supply it
- **AND** no default environment is substituted

### Requirement: Provider detection is confined to the worktree root

Detection SHALL check the worktree root only, and SHALL NOT search parent
directories.

#### Scenario: Monorepo subdirectory

- **WHEN** a sibling or parent directory contains a provider signature file and
  the worktree root does not
- **THEN** that file is not detected
- **AND** resolution continues to the next step in the order

#### Scenario: File outside the worktree

- **WHEN** a signature file exists outside the worktree
- **THEN** it is never used

### Requirement: Ambiguous detection resolves deterministically

When several signature files are present, a documented precedence order SHALL
decide, without prompting.

#### Scenario: Multiple provider files

- **WHEN** more than one signature file is present
- **THEN** the highest-precedence provider is used
- **AND** the choice and the alternatives found are reported

#### Scenario: Ambiguity in a non-interactive context

- **WHEN** resolution happens with no interactive terminal
- **THEN** the same precedence applies
- **AND** no prompt is issued

### Requirement: An environment can be supplied without any project file

Packages SHALL be specifiable on the command line, producing a usable
environment for a repository containing no provider configuration.

#### Scenario: Repository with no environment configuration

- **WHEN** packages are supplied for a repository with no manifest and no
  signature file
- **THEN** an environment containing them is created
- **AND** a command runs inside it, sandboxed

#### Scenario: Nothing is written to the repository

- **WHEN** an ad-hoc environment is used
- **THEN** the repository working tree is unmodified
- **AND** any state the mode needs is kept outside it

### Requirement: Ad-hoc environments are not claimed to be reproducible

Where an environment is assembled from supplied packages rather than a lock, the
tool SHALL say so, and MAY offer to write a manifest capturing what resolved.

#### Scenario: Ad-hoc run completes

- **WHEN** an ad-hoc environment is used
- **THEN** it is reported as not reproducible
- **AND** the tool offers to write a manifest reflecting what resolved

#### Scenario: Manifest is written

- **WHEN** the user accepts
- **THEN** a manifest is written only on explicit acceptance
- **AND** subsequent runs resolve from it

### Requirement: Manifestless mode applies a stricter default policy

Without a manifest, the default policy SHALL be narrower than the manifest
path's default.

#### Scenario: No manifest

- **WHEN** an environment resolves without a manifest
- **THEN** the worktree is writable, other paths are denied, and network access
  is minimal
- **AND** the policy in force is reported

#### Scenario: Project needs more access

- **WHEN** a repository requires access the strict default denies
- **THEN** the denial is reported with the path involved
- **AND** the remedy offered is declaring it, not widening the default

### Requirement: Failures distinguish project errors from tool errors

When a detected or supplied environment fails to resolve, the failure SHALL name
what was detected and report the provider's own error as such.

#### Scenario: Detected configuration does not evaluate

- **WHEN** a detected provider file fails to resolve
- **THEN** the message names the detected file, the provider, and the provider's
  error
- **AND** it is not presented as an internal failure

#### Scenario: Supplied package does not exist

- **WHEN** a supplied package cannot be resolved
- **THEN** the package and the provider that rejected it are named

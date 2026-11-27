## ADDED Requirements

### Requirement: The base runtime namespace is defined
devcroft SHALL define a fixed, versioned set of **base names** for shell and
userland: `sh`, `env`, and the userland names listed in the base-runtime
documentation. For each base name the namespace SHALL have two lookup forms: by
absolute path (`/bin/sh`; `/usr/bin/env`; `/bin/<name>` and `/usr/bin/<name>`
for each utility) and by name through the sandbox's `PATH`. The set SHALL be
published in the documentation and SHALL change only together with a recorded
revision of the inventory.

#### Scenario: The inventory is published
- **WHEN** a user reads the base-runtime documentation
- **THEN** it lists every base name and both lookup forms for each

#### Scenario: A name outside the set is not part of the namespace
- **WHEN** a sandbox executes `/usr/bin/python3`
- **THEN** that path is not reported as part of the base runtime namespace

### Requirement: Base-name provenance is recorded at up
`up` SHALL record, for every base name and each lookup form, exactly one
provenance: `closure:<path>` when it resolves inside the provider's closure,
`provider:<name>:<path>` when it resolves to an artifact provider's own
install, `host:<path>` when it resolves to a host file, or `absent` when it
does not resolve. The record SHALL be stored in the sandbox's metadata and
SHALL be computed from the same resolution the backend profile and the view
are built from, never recomputed from a different source.

#### Scenario: A nix sandbox on Linux
- **WHEN** `up` brings up a nix sandbox on Linux
- **THEN** `sh` by `PATH` and `/bin/sh` are recorded as `closure:` paths
- **AND** no base name is recorded as `host:`

#### Scenario: A pixi sandbox without a shell in its environment
- **WHEN** `up` brings up a pixi sandbox whose locked environment contains no `sh` or `bash`
- **THEN** `/bin/sh` and `sh` are recorded as `host:` with the host shell's path

#### Scenario: A macOS sandbox and an absolute path the policy refuses
- **WHEN** `up` brings up a closure-tier sandbox on macOS
- **THEN** `/bin/sh` is recorded as `absent`, because the sandbox cannot execute it

### Requirement: Provenance is inspectable
`policy --render` and `status` SHALL show the provenance record for the base
runtime namespace, from the stored metadata. A fleet agent SHALL record and
show the same for its own view.

#### Scenario: Rendered with the policy
- **WHEN** a user runs `devcroft policy --render` for an up sandbox
- **THEN** the output contains one line per base name and lookup form with its provenance

#### Scenario: Shown by status
- **WHEN** a user runs `devcroft status`
- **THEN** it summarises how many base names resolve per provenance kind and names every `host:` entry

### Requirement: Host provenance is surfaced once at up
When any base name resolves to `host:`, `up` SHALL print exactly one warning
naming each such base name and its host path. It SHALL NOT refuse to start on
this ground.

#### Scenario: Warned, not refused
- **WHEN** `up` resolves `sh` to the host shell for a mise sandbox
- **THEN** one warning names `sh` and the host path
- **AND** the sandbox starts

### Requirement: The provenance report matches execution
For every provider row the test suite runs, a test SHALL execute each recorded
base name in both lookup forms inside the sandbox and SHALL assert that what
executes matches the record: the recorded path runs for `closure:`,
`provider:` and `host:`, and execution fails for `absent`. On rows that satisfy
the invariant, the test SHALL also assert that no entry is `host:`.

#### Scenario: A report that lies fails the test
- **WHEN** a base name is recorded `absent` but executes inside the sandbox
- **THEN** the provenance test fails naming the base name and the lookup form

## ADDED Requirements

### Requirement: Pinned hermit sources
The hermit provider SHALL require every manifest source in `bin/hermit.hcl` to
be a git source pinned to an immutable revision, verified before any hermit
subprocess. Unpinned sources, `file://` sources and `env:///` sources SHALL be
refused by source at layer `provider`, exit code 3.

#### Scenario: Source following HEAD
- **WHEN** `sources` lists the default manifest repository without a pin
- **THEN** `up` fails before any install and names the source

#### Scenario: Project-local manifests
- **WHEN** `sources` includes `env:///packages`
- **THEN** `up` refuses the source and states that local manifests are project code

### Requirement: hermit packages without command triggers or checksums are refused
Before any install, the provider SHALL read each selected package's manifest
at the pinned revision and refuse, by package, any package with a `run` action
on any trigger, any `exec` trigger, or no checksum for the current platform.

#### Scenario: Unpack trigger runs a command
- **WHEN** a selected package's manifest runs a command on `unpack`
- **THEN** `up` refuses the package, naming it and the trigger, before downloading it

#### Scenario: Sentinel archive
- **WHEN** an admitted package's executables would write a marker if run
- **THEN** after `up` the marker is absent from the host

### Requirement: hermit environment captured as data
The provider SHALL capture the environment from hermit's printed variables
without a shell, SHALL never run hermit's shell hook or `bin/activate-hermit`,
and SHALL run hermit with its state inside a devcroft-owned object and no
inherited `HERMIT_*` variable.

#### Scenario: Ambient hermit state does not change the result
- **WHEN** two callers differ in their hermit cache and `HERMIT_*` variables
- **THEN** the same project inputs produce the same object identity and environment

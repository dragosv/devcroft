## ADDED Requirements

### Requirement: Locked mise resolution
The mise provider SHALL require, for every selected tool, an admitted backend
and a lock entry carrying a URL and a checksum for the current platform,
verified by devcroft before any mise subprocess. It SHALL leave the project's
manifest and lock bytes unchanged. Missing, incomplete or unadmitted inputs
SHALL fail at layer `provider`, exit code 3, before keeper startup, naming the
tool, backend, platform and a corrective hint.

#### Scenario: Cold locked restore
- **WHEN** a supported project starts with an empty artifact store
- **THEN** the exact locked artifacts are verified and materialized once
- **AND** exec and SSH sessions use them without any later mise invocation

#### Scenario: Lock entry without a checksum
- **WHEN** a selected tool's lock entry lacks a URL or a checksum for this platform
- **THEN** `up` fails before any download and names the tool and the missing field
- **AND** no substitute version is resolved

#### Scenario: Unadmitted backend
- **WHEN** a tool uses a plugin backend, a source build or any backend outside the admitted list
- **THEN** `up` refuses the tool and backend with the reason, before running its install code

### Requirement: Host-side mise sees tools only
Host-side `mise` SHALL be invoked only against a derived configuration that
devcroft writes, containing `[tools]` and the settings devcroft itself sets,
beside byte copies of the lock inputs. The project's `[env]`, templates, hooks
and tasks SHALL NOT be present in it. The subprocess environment SHALL be
constructed from an explicit allowlist with an isolated `HOME` and with every
mise directory, including the one installs are written to, inside devcroft's
own object; no ambient `MISE_*` variable SHALL be inherited.

#### Scenario: Ambient mise state does not change the result
- **WHEN** two callers differ in global mise config, `MISE_ENV`, other `MISE_*` variables and default-package files in their home directories
- **THEN** the same project inputs produce the same object identity, tools and environment

#### Scenario: Nothing is written to the user's mise directories
- **WHEN** `up` materializes a project
- **THEN** the user's own mise installs, cache, state and trust records are unchanged

### Requirement: Host-side materialization executes nothing it downloaded
A backend SHALL be admitted only if installing through it on the host executes
no downloaded artifact, as measured for the qualified mise version. A backend
that does SHALL be refused by name. `policy --render` SHALL state what
resolution runs host-side.

#### Scenario: Backend runs the tool after extracting it
- **WHEN** qualification finds that a backend executes the installed tool during installation
- **THEN** that backend is not admitted unless a setting in the derived configuration removes the step
- **AND** that setting is part of the admitted profile

#### Scenario: Sentinel archive
- **WHEN** a locked artifact's executables would write a marker if run
- **THEN** after `up` the marker is absent from the host

### Requirement: The project's mise environment runs confined, at the project root
The provider SHALL interpret the project's `[env]` itself and run it once, in
declaration order, in confined provisioning, with paths and `{{config_root}}`
resolved against the manifest's directory. It SHALL refuse by key every
directive and template it does not carry. The run SHALL use the runtime
filesystem and only the provisioning network policy; a failure SHALL prevent
keeper startup with provisioning attribution.

#### Scenario: Source directive
- **WHEN** an `_.source` script exports a variable and writes a marker in the project
- **THEN** neither happens during host-side capture
- **AND** the script runs once in confined provisioning and sessions inherit the export

#### Scenario: Declaration order
- **WHEN** a literal is declared after a source script that exports the same name
- **THEN** sessions see the literal

#### Scenario: Project-relative values
- **WHEN** `[env]` uses a relative `_.path` entry or `{{config_root}}`
- **THEN** the value names the real project root, never the artifact store

#### Scenario: Unsupported template
- **WHEN** a value uses a template function or variable other than `{{config_root}}`
- **THEN** `up` fails at layer `provider`, exit code 3, naming the key, without evaluating it

### Requirement: mise objects are immutable, shared by recipe and reclaimed
The provider SHALL install into `~/.local/share/devcroft-artifacts/mise/` at
each tool's final path, under an exclusive per-object lock, removing any
directory that lacks a completion record before installing and writing that
record atomically only after verification. Object identity SHALL cover the
derived configuration, lock inputs, target, mise version and adapter version.
A sandbox's recorded state SHALL be its reference to an object; `rm` and a
recreation that changes the object SHALL remove every object no sandbox
references, and SHALL NOT remove one a concurrent `up` is acquiring. `down`
SHALL remove nothing.

#### Scenario: Concurrent consumers
- **WHEN** two sandboxes resolve the same recipe concurrently
- **THEN** neither observes a partial installation and both execute the same object

#### Scenario: Interrupted installation
- **WHEN** installation dies before the completion record is written
- **THEN** a later `up` removes that directory under the object lock and installs again

#### Scenario: Last reference removed
- **WHEN** one of two sandboxes sharing an object is removed, then the other
- **THEN** the object survives the first `rm` and is removed by the second

#### Scenario: Same version, different inputs
- **WHEN** a backend, option, target or lock entry changes without the tool's version string changing
- **THEN** a different object is used

### Requirement: mise environment identity and staleness
The system SHALL fingerprint the manifest, the lock and its sidecars, every
dotenv and source file `[env]` names, the adapter version and the target,
treating an absent referenced file as a distinct input rather than an error.
It SHALL capture environment changes and removals against the canonical base
environment, preserving existing manifest overrides and explicit forwarding.

#### Scenario: Source script edited or deleted
- **WHEN** a consumed script changes or disappears after `up`
- **THEN** `status` reports `env: stale`, naming the mise manifest, and not `unknown`
- **AND** existing sessions keep their resolved environment

#### Scenario: Caller secrets do not enter the environment
- **WHEN** the invoking shell holds an undeclared credential or extra `PATH` entries
- **THEN** they are absent from the captured environment and from diagnostics

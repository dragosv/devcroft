## ADDED Requirements

### Requirement: Locked proto resolution under immutable mode
The proto provider SHALL require, for every selected tool, a `.protolock`
record for the current os, arch, libc and variant that carries a checksum,
verified by devcroft before any proto subprocess. It SHALL install only with
proto's immutable-lockfile mode and SHALL leave the project's `.prototools`
and `.protolock` bytes unchanged. A missing record, a record without a
checksum, or a lock format outside the qualified range SHALL fail at layer
`provider`, exit code 3, naming the tool, the platform and the missing field.

#### Scenario: Record only for other platforms
- **WHEN** the lock records a tool's version for `arm64-macos` only and `up` runs on `x64-linux`
- **THEN** `up` fails before any download and names the tool and `x64-linux`
- **AND** no version is resolved remotely

#### Scenario: Record without a checksum
- **WHEN** a record was written by a source build and has no checksum
- **THEN** `up` refuses the tool by name with the reason

### Requirement: Plugins are pinned before proto loads them
The proto provider SHALL admit only plugins whose code is pinned, either by the
qualified proto release or by an exact version and checksum that devcroft
verifies before proto loads the plugin. Registry locators, unpinned
`github://` or `https://` locators and third-party plugins outside the admitted
set SHALL be refused by locator at layer `provider`.

#### Scenario: Unpinned plugin locator
- **WHEN** `.prototools` declares `[plugins.tools] foo = "github://org/foo"` without a pin
- **THEN** `up` refuses `foo` and names the locator, before any subprocess

### Requirement: proto materialization executes nothing it downloaded
A tool SHALL be admitted only if installing it through the qualified proto
release executes no downloaded artifact, builds nothing from source and
invokes no system package manager, as measured. Every other tool SHALL be
refused by name. `[settings.build]` SHALL be refused by key.

#### Scenario: Tool that would build from source
- **WHEN** a selected Ruby version has no prebuilt for this platform
- **THEN** `up` refuses it and states that proto would build it from source host-side

#### Scenario: Sentinel archive
- **WHEN** a locked artifact's executables would write a marker if run
- **THEN** after `up` the marker is absent from the host

### Requirement: proto sees a derived configuration and an allowlisted environment
Host-side proto SHALL run only against a configuration that devcroft writes,
holding `[tools]`, the admitted plugins and the lockfile setting, with
`PROTO_HOME` inside a devcroft-owned object, an isolated `HOME` and no
inherited `PROTO_*` variable. Configuration found in parent directories or in
the user's home SHALL NOT influence the result.

#### Scenario: Ambient proto state does not change the result
- **WHEN** two callers differ in `~/.prototools`, `~/.proto` contents and `PROTO_*` variables
- **THEN** the same project inputs produce the same object identity and environment

### Requirement: proto environment captured as data
The provider SHALL capture the environment from proto's machine-readable
activation output and SHALL run nothing from the project to obtain it. It
SHALL apply the project's `[env]` literals itself, and SHALL refuse `[env]
file`, `[shell]` and `[shell.aliases]` by key.

#### Scenario: Dotenv file declared
- **WHEN** `.prototools` declares `[env] file = ".env"`
- **THEN** `up` fails at layer `provider` naming the key and pointing at `[env.vars]` in `devcroft.toml`

## ADDED Requirements

### Requirement: The guest environment comes from the project's provider

A VM instance SHALL take its environment from the provider and lock that the
project's `devcroft.toml` names, resolved for the guest's Linux target inside a
preparation VM. The guest SHALL NOT use a bundle's fixed workload image as the
project's toolchain. A bundle MAY supply devcroft's own runtime, kernel, init and
the pinned provider tools that preparation runs.

#### Scenario: A flox project starts in VM mode

- **WHEN** `devcroft up --vm` runs for a project whose manifest names `flox`
- **THEN** the guest's `PATH` resolves the packages that project's lock lists for
  `aarch64-linux`
- **AND** no executable from the bundle's workload image precedes them

#### Scenario: The same manifest runs natively and in a VM

- **WHEN** one project is brought up natively and, under another name, in VM mode
- **THEN** both report the same provider, lock identity and package set, differing
  only in the target system

#### Scenario: An artifact-tier project starts in VM mode

- **WHEN** `devcroft up --vm` runs for a project whose manifest names `mise` or
  `pixi`
- **THEN** the guest's tools are the ones that provider's lock pins for
  `linux-aarch64`, installed by the provider inside the preparation VM
- **AND** `up` and `status` show the `artifact` guarantee tier, as they do natively

#### Scenario: Every provider has a VM outcome

- **WHEN** the manifest names any provider devcroft accepts natively (`nix`,
  `flox`, `devbox`, `devenv`, `mise`, `pixi`, `swift`)
- **THEN** `up --vm` either resolves it for the guest or refuses it by name, and
  `swift` is always refused as Darwin-only

### Requirement: The tooling layer follows the project into the guest

A tooling layer (`up --tools` or the user configuration's `[tools]`) SHALL be
resolved for the guest target inside the preparation VM and composed into the
guest's `PATH` after the project's environment, as native composes it. A layer
that cannot be resolved for the target SHALL be refused by name.

#### Scenario: A user tooling layer in VM mode

- **WHEN** the user configuration names a `nix` tooling layer and `up --vm` runs
- **THEN** the layer's tools are on the guest's `PATH` after the project's, and
  `status` names the layer

### Requirement: A provider the guest target cannot use is refused by name

When the provider, its lock or one of its packages cannot be resolved for the
guest target, `up --vm` SHALL fail at layer `provider`, naming the provider and
what is missing. It SHALL NOT substitute the bundle's tools, re-lock silently or
fall back to the native runtime.

#### Scenario: The lock covers only Darwin

- **WHEN** a project's lock has no `aarch64-linux` entries
- **THEN** `up --vm` exits with the provider exit code and names the missing target
- **AND** no guest session starts

#### Scenario: A provider is not yet qualified for the VM path

- **WHEN** the manifest names a provider the VM path has not requalified
- **THEN** `up --vm` refuses, saying that provider is not yet supported in VM mode
- **AND** the message differs from the one for a provider that is out of scope

### Requirement: The guest shell resolves in the native order

The guest's shell SHALL resolve in the order native uses: one on the
environment's `PATH` inside the provider's closure, then a `bin/sh` from the
closure's requisites, then devcroft's runtime `sh` at a recorded generation. SSH
login sessions, `devcroft shell`'s fallback and service commands SHALL use that
shell. A shell from the guest image's own base SHALL NOT be selected ahead of
these.

#### Scenario: The environment provides bash

- **WHEN** the provider's closure puts `bash` and `sh` on `PATH`
- **THEN** the guest's recorded shell is that closure path, as it is natively

#### Scenario: The environment provides no shell

- **WHEN** the provider's environment and closure contain no `sh`
- **THEN** the guest's shell is devcroft's runtime `sh`, recorded with its generation

### Requirement: The guest base userland matches native resolution

For every name in the base-runtime list (`docs/base-runtime.md`), the guest SHALL
resolve by name and by absolute path in the same order as native: the provider's
executables first, then devcroft's runtime utilities at the same generation, and
otherwise absent. The guest SHALL carry devcroft's runtime built for its target,
at the generation the host's devcroft would use natively.

#### Scenario: The environment declares GNU coreutils

- **WHEN** the provider's environment includes `coreutils`
- **THEN** `ls`, `cat` and the rest resolve to that closure in the guest

#### Scenario: A base name has no source

- **WHEN** neither the environment nor devcroft's runtime provides `grep`
- **THEN** `grep` is absent in the guest, as it is natively, rather than resolving to
  a binary from the guest image

### Requirement: The provenance record is the same for both runtimes

`up` and `status` SHALL report, for a VM instance, the same per-name provenance
record native reports (`closure:`, `provider:<name>:` for mise and pixi,
`runtime:gen-N:`, absent with its reason),
computed from what the guest is actually given. A test SHALL run every base name
in both lookup forms inside a real guest and check the record against it.

#### Scenario: Comparing records

- **WHEN** the same project runs natively and in a VM
- **THEN** each base name's provenance kind matches between the two records

#### Scenario: The record disagrees with the guest

- **WHEN** a name resolves in the guest to a source the record does not show
- **THEN** the qualification test fails

### Requirement: Sessions start in the matching guest directory

`exec`, `shell` and `ssh` SHALL start in the guest path that corresponds to the
host working directory relative to the project root, as native starts in the host
working directory. A host directory outside the project SHALL start at the
project root and say so.

#### Scenario: Running from a subdirectory

- **WHEN** `devcroft exec -- pwd` runs from `<project>/crates/core`
- **THEN** it prints the guest path for `crates/core`

### Requirement: The workspace either matches native or states its difference

Natively a session sees the project directory live. A VM instance SHALL either
present the project directory with the same visibility of host edits, deletions
and `.git`, or SHALL state at `up`, `status` and in the documentation exactly how
its workspace differs. Whichever model ships SHALL NOT silently drop host
deletions, SHALL NOT exclude `.git` without saying so, and SHALL NOT overwrite a
host file changed since the last transfer without detecting the conflict.

#### Scenario: A file deleted on the host

- **WHEN** a file is deleted on the host and the workspace is next brought in line
- **THEN** it is deleted in the guest too, or the difference is reported

#### Scenario: Both sides changed a file

- **WHEN** a file changed on the host and in the guest since the last transfer
- **THEN** the transfer stops on that file and names it, rather than overwriting
  either copy

### Requirement: Destroying a VM workspace never loses work silently

`rm` and `up --recreate` on a VM instance SHALL name any guest workspace changes
not yet transferred to the host before deleting the workspace disk, and SHALL
require `--yes` when stdout is not a terminal, as native destructive commands do.

#### Scenario: Recreating with unsynced changes

- **WHEN** `devcroft up --vm --recreate` runs and the guest workspace holds changes
  the host does not have
- **THEN** the command lists them and stops unless the user confirms

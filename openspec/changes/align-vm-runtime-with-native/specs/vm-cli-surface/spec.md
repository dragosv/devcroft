## ADDED Requirements

### Requirement: Every VM command and flag is discoverable

`USAGE` and each command's own usage line SHALL list every VM command and flag a
user can type, and `tests/cli_help_and_version.rs` SHALL check subcommands and
flags as well as top-level commands.

#### Scenario: A flag missing from help

- **WHEN** a VM flag such as `--sync` or `--vm-memory` is accepted by the parser
- **THEN** it appears in `USAGE` and in `up`'s usage line, or the help test fails

### Requirement: One spelling for one choice

The runtime choice SHALL have one documented spelling. Accepted aliases SHALL be
listed as aliases. The name of the native path SHALL match the term the
documentation uses for it. Grants, bundle installation, workspace transfer and
disk export SHALL each be grouped under a command whose name says what it does.

#### Scenario: Choosing a runtime

- **WHEN** a user reads `USAGE` to start a VM sandbox
- **THEN** one flag is shown for it, with any alias marked as such

### Requirement: Missing prerequisites say how to get them

When VM mode cannot start because the host is unqualified, the artifact bundle is
missing or a grant is absent, the error SHALL name the next command to run.
`doctor` SHALL check the VM prerequisites `runtime status` reports.

#### Scenario: No bundle installed

- **WHEN** `devcroft up --vm` runs with no trusted artifact manifest installed
- **THEN** the error names how to install a bundle and where artifacts come from

#### Scenario: Running doctor

- **WHEN** `devcroft doctor` runs on a qualified macOS ARM64 host without a bundle
- **THEN** it reports the missing bundle as a VM-mode finding

### Requirement: Status shows where the VM's environment comes from

`status` for a VM instance SHALL show the provider and lock identity, the shell
and base-userland provenance summary, and the workspace transfer state (last
transfer in each direction and whether the sides differ).

#### Scenario: Checking a VM sandbox

- **WHEN** `devcroft status` runs for a VM instance
- **THEN** it prints the provider, the shell's provenance and the workspace state

### Requirement: VM mode is described with its limits

Before the `microvm` branch merges, README, `docs/known-gaps.md`,
`docs/threat-model.md` and `AGENTS.md` SHALL describe VM mode as an opt-in path,
with each of its differences from native and each environment variable that
changes its behavior (such as `DEVCROFT_VM_ROLLBACK`).

#### Scenario: Reading the README on the branch

- **WHEN** a user reads the README on a build that accepts `--vm`
- **THEN** it does not say devcroft uses no VM without naming the opt-in VM mode

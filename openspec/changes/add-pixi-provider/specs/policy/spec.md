## ADDED Requirements

### Requirement: Inspectable Pixi artifact and runtime grants
The system SHALL render the Pixi prefix grant and every host-runtime grant
with origin `provider:pixi`. It SHALL preserve baseline denials and SHALL NOT
grant `/usr/bin`, any other host program directory beyond the shell's own on
macOS, the user's Pixi home or cache, or any write access. Host-runtime
requirements SHALL be established by reading executables, never by running
them, starting from the environment's `bin/` entries and following their
needed libraries. Each qualified profile carries a fixed host inventory. An
executable needing a host library outside that inventory SHALL NOT cause it
to be granted; it SHALL be named, with the library, in one `up` warning, and
`up` SHALL proceed. A system requirement in the lock SHALL NOT by itself grant
a device, driver or system service.

#### Scenario: Native dependency with host runtime
- **WHEN** a qualified environment needs the host's loader and libc
- **THEN** those files appear in `policy --render` with origin `provider:pixi` and the workload runs
- **AND** devcroft's own credentials and data directory remain inaccessible

#### Scenario: An executable needs a library outside the inventory
- **WHEN** an environment's `bin/` holds an executable that needs a host library the profile does not carry, such as tk's `wish` needing `libX11`
- **THEN** `up` succeeds, `policy --render` shows no grant for that library, and one warning names the executable and the library

#### Scenario: Host toolchain is not reachable
- **WHEN** a session in a Pixi sandbox on Linux looks up `cc` or `gcc` and the environment provides none
- **THEN** none resolves to a host binary

#### Scenario: Unqualified GPU requirement
- **WHEN** the selected environment requires a driver or device profile that is not qualified
- **THEN** `up` fails at layer `provider`, exit code 3, without exposing a device or a host library tree

### Requirement: Pixi prefixes cannot acquire runtime writers
The system SHALL grant sessions read and execute access to their object's
prefix and nothing else of the store. A manifest write grant that contains,
or is contained by, the Pixi artifact store SHALL be refused at layer
`config`, exit code 2, judged on resolved paths. Mutable runtime state SHALL
belong to the sandbox's own home.

#### Scenario: Runtime write to the prefix
- **WHEN** a session attempts to modify a file in its Pixi prefix
- **THEN** the write is refused and a sibling sandbox's view of the prefix is byte-identical

#### Scenario: Broad write grant
- **WHEN** the manifest grants write access to an ancestor of the artifact store, directly or through a symlink
- **THEN** `up` refuses the grant at layer `config`, exit code 2, naming the store

#### Scenario: Two names share a checkout
- **WHEN** two Pixi sandboxes use the same project root
- **THEN** both read one prefix outside the checkout and each keeps its own home and caches

### Requirement: Pixi shells are supplied with the narrowest grant
The system SHALL resolve an absolute POSIX session shell and a Bash for
activation from the locked prefix when it provides them. Otherwise it SHALL
grant, with origin `provider:pixi`, the host's canonical shell files with the
loader and libraries they need on Linux, and the host shell's directory with
its selector directory on macOS. It SHALL record the session shell
consistently in metadata and rendered policy, and SHALL NOT require the
project to declare a shell.

#### Scenario: Python-only project
- **WHEN** a qualified environment has Python and no shell package
- **THEN** confined activation and SSH login succeed using the granted host shells
- **AND** `policy --render` attributes those grants to `provider:pixi`

#### Scenario: Linux grant is a file
- **WHEN** the host shells are resolved on a Linux host whose `/bin` is a link into `/usr/bin`
- **THEN** the grants name the shells' files, and no directory containing other host programs is granted

## ADDED Requirements

### Requirement: Inspectable mise artifact policy
The system SHALL render every mise installation grant and every host-runtime
grant with origin `provider:mise`. It SHALL preserve baseline denials and SHALL
NOT grant `/usr/bin`, any other host program directory beyond the shell's own
on macOS, the user's mise directories, or any write access. Host-runtime
requirements SHALL be established by reading installed executables, never by
running them, and SHALL be validated before the keeper starts.

#### Scenario: Host-linked tool with explicit libraries
- **WHEN** a qualified tool needs the host's loader and libc
- **THEN** `policy --render` names those files with origin `provider:mise` and the tool runs
- **AND** devcroft's own credentials and data directory remain inaccessible

#### Scenario: Host toolchain is not reachable
- **WHEN** a session in a mise sandbox on Linux looks up `cc`, `gcc` or `ld` and the project declares none
- **THEN** none resolves to a host binary

#### Scenario: Library outside the profile
- **WHEN** an installed executable needs a library found neither in its object nor in the qualified inventory
- **THEN** `up` fails at layer `provider`, exit code 3, naming the tool and the library
- **AND** no directory grant is added to make it work

### Requirement: mise objects cannot acquire runtime writers
The system SHALL grant sessions read and execute access to their object's
installations and nothing more. A manifest write grant that contains, or is
contained by, the mise artifact store SHALL be refused at layer `config`, exit
code 2, judged on resolved paths. Mutable caches SHALL belong to the sandbox's
own home.

#### Scenario: Runtime write to an installed tool
- **WHEN** a session attempts to modify a file under its object
- **THEN** the write is refused and another consumer's copy is byte-identical

#### Scenario: Broad write grant
- **WHEN** the manifest grants write access to an ancestor of the artifact store, directly or through a symlink
- **THEN** `up` refuses the grant at layer `config`, exit code 2, naming the store

### Requirement: mise supplies devcroft's shell with the narrowest grant
The system SHALL resolve an absolute POSIX shell from the object when a tool
ships one. Otherwise it SHALL grant, with origin `provider:mise`, the host
shell's canonical file with the loader and libraries it needs on Linux, and
the host shell's directory with its selector directory on macOS. It SHALL
record the shell consistently in metadata and rendered policy, and SHALL NOT
require the project to declare a shell.

#### Scenario: Project declares tools but no shell
- **WHEN** a qualified mise project starts
- **THEN** SSH login, the shell fallback and supervised commands use the resolved absolute shell
- **AND** `policy --render` attributes its grant to `provider:mise`

#### Scenario: Linux grant is a file
- **WHEN** the host shell is resolved on a Linux host whose `/bin` is a link into `/usr/bin`
- **THEN** the grant names the shell's file, and no directory containing other host programs is granted

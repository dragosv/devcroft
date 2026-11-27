## ADDED Requirements

### Requirement: A sandbox has its own temp directory
Every sandbox that is not in native mode SHALL have a temp directory of its own,
readable and writable by its processes, that no other sandbox's processes may
read or write. `TMPDIR`, `TMP` and `TEMP` SHALL name it in every process devcroft
starts, unless the environment names a directory the sandbox may write. On macOS
it SHALL be mode 0700, created without following links, and the baseline SHALL
NOT grant the host's `/tmp`.

#### Scenario: mktemp works
- **WHEN** a session runs `mktemp` in a macOS sandbox whose environment sets no `TMPDIR`
- **THEN** a file is created in the sandbox's temp directory

#### Scenario: Two sandboxes
- **WHEN** one sandbox writes a file in its temp directory and another lists or reads its own
- **THEN** the second does not see the file

#### Scenario: The host's files
- **WHEN** a macOS sandbox whose manifest does not grant `/tmp` tries to read or remove a file of the host's in `/tmp`
- **THEN** it fails and the file is intact

#### Scenario: A manifest asks for the host's
- **WHEN** a manifest has `filesystem.allow = ["/tmp"]` on macOS
- **THEN** `/tmp` is the host's, and `policy --render` shows it with origin `manifest:filesystem.allow`

### Requirement: Literal temp paths reach it
On macOS, a path under `/tmp` or `/private/tmp` that no grant covers SHALL reach
the sandbox's temp directory in a process that holds devcroft's library, and
SHALL be refused in one that does not. It SHALL never reach the host's.

#### Scenario: A hardcoded path
- **WHEN** a program opens `/tmp/x` for writing
- **THEN** the file is in the sandbox's temp directory, and the host's `/tmp` does not have it

#### Scenario: Both names of a call
- **WHEN** a program renames a workspace file to `/tmp/x`
- **THEN** it is in the sandbox's temp directory

#### Scenario: A parent step is not a way out
- **WHEN** a program opens `/tmp/../tmp/x` or `/private/tmp/./x`
- **THEN** it is the same file as `/tmp/x`

#### Scenario: A process without the library
- **WHEN** a program that dropped the library opens `/tmp/x`
- **THEN** it is refused, and the same file under `$TMPDIR` can be opened

#### Scenario: A project kept there
- **WHEN** the project root is under `/private/tmp`
- **THEN** its paths are its own, not redirected

### Requirement: devenv runtime paths are scoped to the sandbox
On macOS, devcroft SHALL evaluate devenv with a dotfile directory and a
runtime directory specific to each sandbox. The provider SHALL expose
`DEVENV_DOTFILE` and `DEVENV_RUNTIME` to hooks, services and sessions.
The dotfile directory SHALL persist across `down` and `up`.

#### Scenario: Two devenv sandboxes of one checkout
- **WHEN** two sandboxes of one devenv project run their activation hooks
- **THEN** each can use its own `$DEVENV_RUNTIME` and `$DEVENV_DOTFILE/run`
- **AND** starting the second leaves the first sandbox's run link pointing to its own directory

#### Scenario: A script hardcodes devenv's checkout link
- **WHEN** project code directly names `.devenv/run` on macOS
- **THEN** it uses the checkout's entry as usual
- **AND** devcroft documents the migration to `$DEVENV_RUNTIME` or `$DEVENV_DOTFILE/run`

### Requirement: The home and temp directories have stated lifetimes
The sandbox `HOME` SHALL be readable and writable by it, kept across `down` and
`up`, and removed by `rm`. The temp directory SHALL be empty when provisioning
begins, shared with the subsequent keeper, and removed by `rm`.

#### Scenario: Across a restart
- **WHEN** a session writes a file in `HOME` and one in the temp directory, and the user runs `down` and then `up`
- **THEN** the first is there and the second is not

### Requirement: The user database agrees with the home
On macOS, in a process that holds devcroft's library, `getpwuid` and `getpwnam`
for the sandbox's user SHALL answer the sandbox's user name, `HOME` and shell.

#### Scenario: A program that ignores HOME
- **WHEN** a program asks `getpwuid(getuid())` for its home directory
- **THEN** it is the sandbox's `HOME`

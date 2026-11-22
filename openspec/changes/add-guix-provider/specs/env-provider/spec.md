## ADDED Requirements

### Requirement: Pinned guix resolution
The guix provider SHALL resolve only through `guix time-machine` with the
project's channels file passed explicitly, and SHALL require that file to
exist and to pin every channel to a commit. A missing file, a channel without
a commit, or a file that is not a literal list of channel forms SHALL fail at
layer `provider`, exit code 3, before any guix subprocess, naming the channel
or the file.

#### Scenario: Channel without a commit
- **WHEN** `channels.scm` lists the `guix` channel with a URL and no commit
- **THEN** `up` fails before running guix and names the channel
- **AND** the user's own channel configuration is never consulted

#### Scenario: Same pin, same closure
- **WHEN** two hosts run `up` on the same `manifest.scm` and `channels.scm`
- **THEN** the captured environment names the same store paths

### Requirement: guix environment captured as data
The provider SHALL obtain the environment from guix's printed search-path
definitions with the environment pure, and SHALL parse them without a shell.
A line outside the assignment grammar SHALL fail at layer `provider`, quoting
the line. No activation code SHALL run, host-side or inside the sandbox.

#### Scenario: Unexpected output line
- **WHEN** the printed definitions contain a line that is not an assignment
- **THEN** `up` fails at layer `provider` and quotes the line

### Requirement: guix closures protected from garbage collection
Each sandbox's profile SHALL be registered as a GC root under devcroft's state
directory at `up`, and removed at `rm`. A missing root SHALL be reported by
`status` as broken.

#### Scenario: Collection while running
- **WHEN** `guix gc` runs while a guix sandbox is up
- **THEN** the sandbox's closure is still present and sessions keep working

### Requirement: guix is Linux-only
On a host without Guix support the provider SHALL fail at layer `provider`,
exit code 3, stating that Guix does not run on this platform. It SHALL never
fall back to another provider.

#### Scenario: macOS
- **WHEN** `provider = "guix"` and `up` runs on macOS
- **THEN** `up` fails at layer `provider` naming the platform

## ADDED Requirements

### Requirement: A dedicated, non-root identity inside every sandbox
Every process devcroft starts inside a sandbox — sessions, `exec`, SSH
sessions, devcroft hooks, confined provider activation, services and fleet
agents — SHALL run under the sandbox's dedicated identity: the user name from
`[identity] user` (default `devcroft`), a non-zero uid and gid, and the sandbox
`HOME`. `getpwuid` of that uid, `id -un`, `$USER`, `$LOGNAME` and `$HOME` SHALL
agree. No such process SHALL run with uid 0 inside the sandbox. A devcroft
setup helper that holds uid 0 inside a namespace SHALL hold it only before the
restriction is applied, and SHALL NOT run project code while it does.

#### Scenario: The default name
- **WHEN** a session in a sandbox without an `[identity]` table runs `id -un` and `echo $HOME`
- **THEN** it prints `devcroft` and the sandbox `HOME`

#### Scenario: Never root
- **WHEN** a hook, a service and a session each run `id -u`
- **THEN** each prints a non-zero uid

#### Scenario: A configured name
- **WHEN** the manifest sets `[identity] user = "dev"`
- **THEN** `id -un` prints `dev` in every session of that sandbox

### Requirement: Separate host identity is the default
In mode `separate`, the default, the sandbox identity SHALL map to a host uid
and gid different from those of the user who ran `up`, and distinct from those
of every other sandbox the user has up. On Linux the mapping SHALL use a user
namespace into the user's delegated subordinate ranges. On macOS it SHALL use a
dedicated local account per sandbox, prepared by devcroft's privileged helper.
A host identity SHALL NOT be reassigned to another sandbox while files or
processes it owns remain.

#### Scenario: A file created in separate mode
- **WHEN** a session in a `separate` sandbox creates a file in the workspace
- **THEN** on the host the file's owner is the sandbox's host uid, not the user's

#### Scenario: Two sandboxes
- **WHEN** two `separate` sandboxes are up for the same user
- **THEN** their host uids differ from each other and from the user's

### Requirement: Shared host identity is an explicit alternative
In mode `shared`, selected by `[identity] mode = "shared"`, the sandbox identity
SHALL map to the host uid and gid of the user who ran `up`. On Linux the
dedicated name and uid inside SHALL still apply, through a user namespace. On
macOS, where no such mapping exists, the inside identity SHALL be the host
user's, and `up` SHALL state that the dedicated name is not available.

#### Scenario: Shared mode on Linux
- **WHEN** a `shared` sandbox on Linux creates a file in the workspace
- **THEN** inside, `id -un` prints the dedicated name, and on the host the file belongs to the user

#### Scenario: Shared mode on macOS
- **WHEN** `up` brings up a `shared` sandbox on macOS
- **THEN** it states once that the sandbox runs as the host user's name and uid

### Requirement: No silent fallback between modes
When mode `separate` is selected and a precondition is missing — user
namespaces unavailable, no delegated subordinate range, the mapping helpers
absent, or on macOS the helper not installed — `up` SHALL fail at layer
`backend`, exit code 4, naming each missing precondition and the `shared`
alternative, and SHALL NOT start the sandbox under the user's uid. `devcroft
doctor` SHALL report the same preconditions. On a platform where devcroft does
not yet implement `separate`, `up` SHALL state the mode it uses instead.

#### Scenario: No subordinate range
- **WHEN** `up` runs in `separate` mode on a Linux host with no `/etc/subuid` entry for the user
- **THEN** it fails at layer `backend` with exit code 4, naming the missing range and `[identity] mode = "shared"`

### Requirement: The sandbox identity does not own the environment
In both modes, the sandbox identity SHALL NOT be able to modify: the provider's
closure and artifact-store objects, devcroft's runtime, the namespace
directory, the macOS redirect library, the compiled policy and other artifacts
under `<project>/.devcroft/<name>/` except the sandbox `HOME` and the `run`
directory, which holds only what the sandbox itself produces: the service
supervisor's socket and log, and an agent's report. It SHALL be able to write
the workspace, the sandbox `HOME` and `run`. In `separate` mode this SHALL
hold by ownership as well as by policy.

#### Scenario: The runtime cannot be replaced from inside
- **WHEN** a session tries to overwrite an entry of the namespace directory or the runtime directory
- **THEN** the write fails, and the next exec of that base name runs the pinned target

#### Scenario: The workspace stays writable
- **WHEN** a session edits a file in the workspace
- **THEN** the edit succeeds

### Requirement: Workspace access in separate mode is explicit and reversible
In mode `separate`, `up` SHALL give the sandbox identity access to the workspace
through an explicit grant that leaves the user able to read and write
everything the sandbox creates, and `rm` SHALL remove it. devcroft SHALL NOT
change the owner of the user's existing files and SHALL NOT make any path
world-writable. Fleet clones SHALL be created owned by the agent's identity.
Version control run inside the sandbox SHALL treat the workspace as trusted.

#### Scenario: The user edits what the sandbox created
- **WHEN** a `separate` sandbox creates a file and the user then edits it from the host
- **THEN** the edit succeeds without `sudo`

#### Scenario: rm removes the grant
- **WHEN** the user runs `devcroft rm` for a `separate` sandbox
- **THEN** the sandbox identity's access entries are removed from the workspace

#### Scenario: git inside the sandbox
- **WHEN** a session runs `git status` in a workspace owned by the user
- **THEN** git does not refuse with a dubious-ownership error

### Requirement: The control plane stays with the user
The CLI, host-side provider resolution and materialization, the state
directory and the listeners `up` creates SHALL remain under the user's
identity. The keeper SHALL receive the listeners, its log and any state it must
write as descriptors opened before it switches identity, so no directory of
the user's needs to become writable by the sandbox identity.

#### Scenario: The user reaches a separate sandbox
- **WHEN** the user runs `devcroft shell` or `ssh <name>.devcroft` for a `separate` sandbox
- **THEN** the session starts, and the socket and state directories are still owned by the user and not accessible to other users

### Requirement: Identity is inspectable
`status` and `policy --render` SHALL show the inside identity (name, uid, gid),
the host identity, the mode, and on Linux the mapping.

#### Scenario: Shown by status
- **WHEN** the user runs `devcroft status` for a `separate` sandbox on Linux
- **THEN** it shows `devcroft` with its inside uid, the host uid it maps to, and `mode: separate`

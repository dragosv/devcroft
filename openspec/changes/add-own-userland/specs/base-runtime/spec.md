## ADDED Requirements

### Requirement: The base runtime is embedded and dispatched by name
The `devcroft` binary SHALL contain the base runtime's utilities and SHALL
dispatch on the basename of `argv[0]` before any other argument parsing: a
basename in the runtime inventory runs that utility, any other basename follows
the normal command path. Each utility SHALL run in its own process through the
upstream wrapper of the pinned uutils version, so localisation, `SIGPIPE`
handling and output flushing behave as upstream. A utility SHALL never run
inside the keeper or the provisioning process. The published command surface
SHALL NOT change.

#### Scenario: Dispatch by argv[0]
- **WHEN** the binary is executed through a link named `ls`
- **THEN** it runs the `ls` utility and exits with its status

#### Scenario: The public surface is unchanged
- **WHEN** a user runs `devcroft ls`
- **THEN** it fails as an unknown command, exactly as before this change

#### Scenario: A closed pipe
- **WHEN** `ls` writes into a pipe whose reader has exited
- **THEN** it terminates by `SIGPIPE` without a panic message

### Requirement: The runtime generation is pinned by the project
Each `devcroft` binary SHALL embed exactly one runtime generation. A generation
SHALL identify the runtime's behaviour: the utility inventory and versions, the
absolute-path mapping rules, and any component later added to the runtime. When
the manifest declares `[runtime] generation` and it differs from the binary's,
`up` SHALL fail at layer `config`, exit code 2, naming both generations. When the
manifest does not declare it, `up` SHALL use the binary's generation and print
it once with the line that pins it. A running keeper SHALL keep the generation
it started with; a different generation SHALL require `up --recreate`.

#### Scenario: Mismatch is refused
- **WHEN** a manifest pins generation 1 and the installed binary embeds generation 2
- **THEN** `up` fails at layer `config` with exit code 2 naming generation 1 and generation 2

#### Scenario: Unpinned project
- **WHEN** a manifest has no `[runtime]` table
- **THEN** `up` uses the binary's generation and prints it with `[runtime] generation = N`

#### Scenario: Upgrade with a live keeper
- **WHEN** the installed binary changes generation while a sandbox is up
- **THEN** `status` reports the sandbox's generation and the installed one, and sessions keep the started generation until `up --recreate`

### Requirement: PATH belongs to the project, then to the runtime
The sandbox `PATH` SHALL be the provider environment's `PATH`, followed by the
runtime's directory, with no host directory after it. A base name the provider
environment supplies SHALL resolve to the provider's executable.

#### Scenario: A project that declares coreutils
- **WHEN** a nix sandbox runs `ls` by name
- **THEN** GNU coreutils' `ls` from the closure runs, not the runtime's

#### Scenario: A minimal flox project
- **WHEN** a flox sandbox whose manifest declares only `python3` runs `ls` by name
- **THEN** the runtime's `ls` runs and provenance records `runtime:gen-N`

### Requirement: An absolute path resolves as its name does
For every base name, `up` SHALL compute one target: the executable the
sandbox resolves for that name considering only the provider environment and
the runtime — the provider's when it supplies one, otherwise the runtime's. For
`sh` the target SHALL be the shell resolved for the sandbox. `up` SHALL build
one namespace directory per sandbox holding one entry per base name that has a
target, and both platforms SHALL resolve the absolute-path namespace from that
directory. `/bin/<name>`, `/usr/bin/<name>` (and `/bin/sh`, `/usr/bin/env`)
SHALL execute the same file as `<name>` looked up through the sandbox `PATH`.
A base name with no target SHALL have no absolute entry. `policy --render`
SHALL show each entry with its target and provenance.

#### Scenario: A nix sandbox
- **WHEN** a nix sandbox executes `/bin/ls` and `ls`
- **THEN** both run GNU coreutils' `ls` from the closure

#### Scenario: A minimal flox sandbox
- **WHEN** a flox sandbox whose environment supplies no coreutils executes `/bin/ls` and `ls`
- **THEN** both run the runtime's `ls`

#### Scenario: A name no source supplies
- **WHEN** a flox sandbox without `awk` executes `/bin/awk`
- **THEN** it fails because no entry exists, and no host `awk` runs

### Requirement: The namespace is mounted into every Linux view
On Linux, every sandbox view, including fleet agent and provisioning views,
SHALL expose the namespace directory's entries at `/bin/<name>` and
`/usr/bin/<name>`, and SHALL NOT contain any other entry under `/bin` or
`/usr/bin`. `/usr/bin/env` SHALL be present in every view, from the provider
environment or from the runtime. Runtime entries SHALL carry origin
`runtime:gen-N`. On a host where the view cannot be constructed, `up` SHALL
refuse rather than start without the namespace.

#### Scenario: An env shebang in a sandbox without coreutils
- **WHEN** a flox sandbox without coreutils executes a script starting `#!/usr/bin/env python3`
- **THEN** the runtime's `env` runs and executes `python3` found through `PATH`

#### Scenario: An absolute path outside the namespace
- **WHEN** a sandbox executes `/usr/bin/python3`
- **THEN** it fails because the path does not exist in the view

#### Scenario: Fleet agents get the same namespace
- **WHEN** a fleet agent in a flox environment executes `/bin/ls`
- **THEN** the same `ls` runs as for `ls` by name in that agent

### Requirement: The runtime cannot be modified from inside the sandbox
The runtime directory, the namespace directory and every other runtime
artifact `up` writes under `<project>/.devcroft/<name>/` SHALL be read-only to
every process inside the sandbox, even though the project root is writable:
on Linux by mounting them read-only in the view, on macOS by a write denial in
the compiled profile that takes precedence over the project grant. Each denial
SHALL be shown by `policy --render`.

#### Scenario: Overwriting a namespace entry fails
- **WHEN** a session runs `ln -sf /tmp/x <project>/.devcroft/<name>/ns/ls`
- **THEN** the write fails, and `/bin/ls` still runs its pinned target

#### Scenario: Removing the runtime fails
- **WHEN** a session runs `rm -rf <project>/.devcroft/<name>/runtime`
- **THEN** the removal fails and the runtime is intact

### Requirement: One common runtime layer, one declared native exception
The runtime namespace SHALL be completed by a single layer applied after
provider resolution, for every provider. A provider SHALL NOT supply a host
shell, host utility or host `env` of its own except by declaring **native** mode
with a reason. Native mode SHALL be shown by `up`, `status` and `policy
--render` together with its reason, and its entries SHALL be recorded with
provenance `native:<provider>`. The `swift` provider SHALL be the only provider
that declares it. A manifest grant of an individual host developer tool to a
non-native sandbox, made to obtain a base name, SHALL be refused naming the
native mode. Until the runtime contains a shell, the common layer SHALL supply
the host's POSIX shell, as a single file grant, to an environment that resolves
none; that entry SHALL be recorded `host:` and warned about at `up`.

#### Scenario: Swift is native and says so
- **WHEN** `up` brings up a swift sandbox
- **THEN** it prints that the sandbox runs in native mode and why
- **AND** the provenance of `/bin/sh` is `native:swift`

#### Scenario: Another provider cannot borrow the host shell silently
- **WHEN** a provider other than swift resolves no shell
- **THEN** the shell comes from the common layer, or `up` reports `host:` until a runtime shell exists, never from a provider-private fallback

### Requirement: The utility inventory is qualified and published
Every utility in the runtime inventory SHALL have recorded qualification
against the options the qualified workloads use, and the inventory SHALL be
published with its upstream project and version. A utility family outside
coreutils (find, diff, grep, sed) SHALL enter the inventory only by a recorded
decision after qualification.

#### Scenario: A family is admitted by decision
- **WHEN** `grep` passes qualification on the recorded options
- **THEN** it enters the inventory with a new generation and the decision is recorded

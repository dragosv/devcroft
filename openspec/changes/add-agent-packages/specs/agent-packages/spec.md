## ADDED Requirements

### Requirement: Agent recipes are owned by devcroft
devcroft SHALL ship agent recipes as data in its release. Each recipe SHALL
pin, per supported platform, every artifact it installs with a URL and a
checksum, its entrypoints, the absolute paths of every executable its chain
starts, its private state variables, its network destinations and its
authentication methods. An unknown recipe name or an unsupported platform
SHALL fail at layer `config`, exit code 2, listing the available recipes.

#### Scenario: Unknown recipe
- **WHEN** the user configuration selects `[agent] name = "nope"`
- **THEN** `up` fails at layer `config` and lists the recipe names

#### Scenario: Recipe without this platform
- **WHEN** a recipe has no artifacts for the host's platform
- **THEN** `up` fails before any download and names the recipe and the platform

### Requirement: Agent materialization executes nothing downloaded
devcroft SHALL verify every artifact's checksum before use and SHALL install
by extraction or by laying out a pinned lockfile, without running any
downloaded code host-side. A lock entry that declares an install script SHALL
refuse the recipe unless the recipe declares it unnecessary. A recipe that
needs code at install SHALL run it only in confined provisioning, writing
only into the staging object.

#### Scenario: Checksum mismatch
- **WHEN** a downloaded artifact does not match its recipe checksum
- **THEN** `up` fails at layer `provider`, nothing is published, and the object id is unchanged

#### Scenario: Sentinel package
- **WHEN** a pinned package's files would write a marker if executed
- **THEN** after `up` the marker is absent from the host

### Requirement: Agent objects are immutable and shared
An installed agent SHALL be published atomically as a read-only object
identified by its recipe and artifacts, shared by every sandbox that selects
it, and removed only when no sandbox references it.

#### Scenario: Two sandboxes, one object
- **WHEN** two sandboxes select the same recipe version
- **THEN** both use one object and neither can write to it

#### Scenario: Last reference removed
- **WHEN** the last sandbox referencing an agent object is removed
- **THEN** the object is swept

### Requirement: The agent's runtime does not reach the project's commands
The agent SHALL start inside the project's sandbox with the project's
environment, through absolute paths into its object, and nothing from its
object SHALL be prepended to `PATH`. Only a launcher named for the agent's own
command SHALL be on `PATH`, after the project's entries.

#### Scenario: Divergent Node versions
- **WHEN** the project's environment provides Node 16 and the agent's recipe provides Node 24
- **THEN** the agent process runs on Node 24
- **AND** a `node --version` the agent runs through its shell tool reports Node 16

#### Scenario: Sentinel on the project's PATH
- **WHEN** the project's `PATH` holds a `node` that writes a marker when run
- **THEN** starting the agent and completing a task that edits a file writes no marker

### Requirement: ACP endpoint
`devcroft agent acp [name]` SHALL start the selected agent's ACP entrypoint
inside the sandbox as a piped session and SHALL pass stdin and stdout through
unmodified, keeping stderr separate. It SHALL start the sandbox first if it is
not running.

#### Scenario: Editor handshake
- **WHEN** an ACP client runs `devcroft agent acp <name>` and sends `initialize`
- **THEN** it receives the agent's ACP response with no devcroft output on stdout

### Requirement: Agent state is private and writable
The agent SHALL write its config, history and cache only under a per-sandbox,
per-recipe state directory reached through the recipe's state variables. The
directory SHALL be removed at `rm`, and the agent SHALL NOT be given a
different `HOME` from the project's commands.

#### Scenario: Agent writes history
- **WHEN** the agent records a session
- **THEN** the files appear under the sandbox's agent state directory and not in the object or the workspace

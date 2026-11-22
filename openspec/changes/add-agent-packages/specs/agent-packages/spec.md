## ADDED Requirements

### Requirement: Agent recipes are owned by devcroft
devcroft SHALL ship agent recipes as data in its release, each with one or
more versions. Each version SHALL pin, per supported platform, every artifact
it installs with a URL and a checksum, its entrypoints, the absolute paths of
every executable its chain starts, its network destinations, its
authentication methods and its sandbox requirements. A user recipe in the
user configuration directory SHALL pass the same validation, with a checksum
on every artifact, and SHALL be marked as a user recipe wherever the recipe
is shown. An unknown recipe or version, or an unsupported platform, SHALL
fail at layer `config`, exit code 2, listing what is available.

#### Scenario: Unknown recipe
- **WHEN** the user configuration selects `[agent] name = "nope"`
- **THEN** `up` fails at layer `config` and lists the recipe names

#### Scenario: Recipe without this platform
- **WHEN** a recipe has no artifacts for the host's platform
- **THEN** `up` fails before any download and names the recipe and the platform

#### Scenario: User recipe without a checksum
- **WHEN** a user recipe declares an artifact without a checksum
- **THEN** `up` fails at layer `config` naming the file and the artifact

#### Scenario: Rollback
- **WHEN** the user pins the previous version of a recipe and runs `up --recreate`
- **THEN** the sandbox runs that version, from its object if still present

### Requirement: Agent materialization executes nothing downloaded
devcroft SHALL verify every artifact's checksum before use and SHALL install
by extraction or by laying out a pinned lockfile, without running any
downloaded code host-side. Extraction SHALL refuse entries that resolve
outside the staging object. A lock entry that declares an install script SHALL
refuse the recipe unless the recipe declares it unnecessary. A recipe that
needs code at install SHALL run it only in confined provisioning, writing
only into the staging object.

#### Scenario: Checksum mismatch
- **WHEN** a downloaded artifact does not match its recipe checksum
- **THEN** the install fails at layer `provider`, nothing is published, and the object id is unchanged

#### Scenario: Sentinel package
- **WHEN** a pinned package's files would write a marker if executed
- **THEN** after the install the marker is absent from the host

#### Scenario: Archive escape
- **WHEN** an artifact contains an entry or a link that resolves outside the staging object
- **THEN** the install fails at layer `provider` naming the entry

### Requirement: Agent objects are immutable, shared and leased
An installed agent SHALL be published atomically as a read-only object
identified by its recipe, version and artifacts, shared by every sandbox that
selects it, and removed only when no sandbox references it. `up` SHALL hold
the agent store's lease whenever an agent is selected, whatever the project's
provider.

#### Scenario: Two sandboxes, one object
- **WHEN** two sandboxes select the same recipe version
- **THEN** both use one object and neither can write to it

#### Scenario: Last reference removed
- **WHEN** the last sandbox referencing an agent object is removed
- **THEN** the object is swept

#### Scenario: Concurrent removal under a closure provider
- **WHEN** a nix project's `up` with an agent runs while another sandbox is removed
- **THEN** the agent object is not swept before this `up` records it

### Requirement: The agent's runtime does not reach the project's commands
The agent SHALL start inside the project's sandbox through absolute paths into
its object, and nothing from its object SHALL be prepended to `PATH`. Only the
launcher directory, inside the object and holding only the agent's own
command, SHALL be on `PATH`, after the project's entries and the tooling
layer. No launcher SHALL use a shell interpreter. The recipe's environment
SHALL NOT set `PATH`, `HOME`, `XDG_*`, `LD_*`, `DYLD_*` or a name the recipe
declares its runtime reads.

#### Scenario: Divergent Node versions
- **WHEN** the project's environment provides Node 16 and the agent's recipe provides Node 24
- **THEN** the agent process runs on Node 24
- **AND** a `node --version` the agent runs through its shell tool reports Node 16

#### Scenario: Sentinel on the project's PATH
- **WHEN** the project's `PATH` holds a `node` that writes a marker when run
- **THEN** starting the agent and completing a task that edits a file writes no marker

#### Scenario: Project sets a runtime variable
- **WHEN** the project's environment sets `NODE_OPTIONS` and the recipe declares it in `requires.runtime_env`
- **THEN** `up` prints one warning naming `NODE_OPTIONS` and the recipe

#### Scenario: Launcher on macOS
- **WHEN** the user runs the agent's command in `devcroft shell` on macOS
- **THEN** it starts, and no shell outside the sandbox's grants is executed

### Requirement: Agent state lives in the sandbox's HOME
The agent SHALL keep its config, history, cache and login in the sandbox's own
`HOME`, never under devcroft's data directory and never in its object. It
SHALL be removed with the sandbox at `rm`.

#### Scenario: Agent writes history
- **WHEN** the agent records a session
- **THEN** the files appear under `<project>/.devcroft/<name>/home` and not in the object or `~/.local/share/devcroft`

#### Scenario: why agrees with the kernel
- **WHEN** `devcroft why` is asked about a file the agent wrote
- **THEN** it reports the write as allowed

### Requirement: The recipe's credentials need no user wiring
A recipe's broker template SHALL be active only in sandboxes that select the
recipe, and SHALL need nothing from the user but the exported key. A user
broker entry for the same host or variable SHALL fail at layer `config`
naming both. A recipe broker SHALL be refused, naming `--no-agent`, when the
project's `network.default` is `"allow"`. A login stored in the sandbox's
`HOME` SHALL be announced at every `up` as readable by every process in the
sandbox.

#### Scenario: Project with an empty allowlist
- **WHEN** a project with `network.allow = []` is brought up with `[agent] name = "claude-code"` and `ANTHROPIC_API_KEY` exported
- **THEN** `up` succeeds and the agent's request reaches the API through the broker

#### Scenario: Another agent's sandbox
- **WHEN** the same user brings up another project with a different recipe
- **THEN** no broker or host of `claude-code` is in that sandbox's policy

#### Scenario: Default allow
- **WHEN** the project's `network.default` is `"allow"` and the recipe brokers its key
- **THEN** `up` fails at layer `config` naming the agent and `--no-agent`

### Requirement: ACP endpoint
`devcroft agent acp` SHALL run on the host, resolve the sandbox whose project
root contains its `cwd` (or the one `--sandbox` names), and start the agent's
ACP entrypoint as a piped session with `cwd` inside the project root. It SHALL
pass stdin and stdout through unmodified and write every devcroft message to
stderr. It SHALL start the sandbox with the user configuration if it is down,
SHALL NOT download, and SHALL NOT recreate a sandbox that runs without that
agent or version.

#### Scenario: Editor handshake
- **WHEN** an ACP client runs `devcroft agent acp` in a project and sends `initialize`
- **THEN** it receives the agent's ACP response with no devcroft output on stdout

#### Scenario: Object not installed
- **WHEN** `agent acp` runs and the recipe's object is absent
- **THEN** it fails at once, naming `devcroft agent install <recipe>`

#### Scenario: Sandbox up without the agent
- **WHEN** the sandbox is up without the selected recipe
- **THEN** `agent acp` fails naming `devcroft up --recreate --agent <recipe>`

#### Scenario: Editor started outside the project
- **WHEN** `agent acp` runs with `cwd = /` and no `--sandbox`
- **THEN** it fails saying no sandbox's project contains the directory

#### Scenario: Two checkouts
- **WHEN** two checkouts of one project are up under different sandbox names
- **THEN** `agent acp` in each checkout attaches to that checkout's sandbox

### Requirement: Interactive run
`devcroft agent run` SHALL resolve the sandbox as `agent acp` does and run the
recipe's interactive argv on a PTY.

#### Scenario: Terminal UI
- **WHEN** the user runs `devcroft agent run` in a terminal
- **THEN** the agent's terminal interface starts with a PTY of the terminal's size

### Requirement: Recipe requirements are checked at up
`up` SHALL check a recipe's declared requirements before restriction. A
missing procfs SHALL refuse the recipe at layer `backend`; a missing declared
command on the sandbox's `PATH` SHALL produce one warning naming it.

#### Scenario: Bun harness on a host without procfs
- **WHEN** a recipe requiring procfs is selected where the sandbox has none
- **THEN** `up` fails naming the recipe and the requirement

## ADDED Requirements

### Requirement: Agent recipes are owned by devcroft
devcroft SHALL ship agent recipes as data in its release, reproducible from
committed repository inputs without requiring `mise`, `npm` or `nix` on the
host at runtime. Each recipe version SHALL pin, per supported platform, every
artifact it installs with a URL and a checksum, its upstream ACP capability,
each component's version, its entrypoints, its relative `state_paths` under
`HOME` (refusing empty, absolute or `..` segments), any `reserved_env` names
stripped on launch, its network destinations, its supported authentication
modes (with `default_auth`), its platform-scoped sandbox requirements, each
artifact's CPU architecture and minimum OS version (the glibc version for a
`gnu` artifact, the macOS version for a Mach-O one) and, for each Linux
artifact, its libc (`gnu`, `musl` or `static`). A recipe name SHALL
be one lowercase component (`[a-z0-9][a-z0-9-]*`), and each entrypoint's
`argv[0]` SHALL be a relative path of normal components that resolves inside
the object, checked before download and again after extraction. A user recipe
in `agents/` beside the resolved user configuration file (so honoring
`XDG_CONFIG_HOME`) SHALL pass the same validation, with a checksum on every
artifact, and SHALL be marked as a user recipe wherever the recipe is shown. An
unknown recipe or version, an unsupported platform, or a host whose libc does
not match or whose OS is older than the declared minimum SHALL fail at layer
`config`, exit code 2, before any download, listing what is available. After
download and before publication, the executable's headers SHALL match what
the recipe declares (ELF `e_machine` or the selected Mach-O slice, and the
minimum OS the binary itself records); a mismatch SHALL fail at layer
`provider` and publish nothing. The published crate SHALL carry the embedded
recipes, and every crate the fetcher and extractor call SHALL be a direct
dependency.

#### Scenario: Unknown recipe
- **WHEN** the user configuration selects `[agent] name = "nope"`
- **THEN** `up` fails at layer `config` and lists the recipe names

#### Scenario: Host older than the recipe's minimum
- **WHEN** a recipe declares a `gnu` artifact needing glibc 2.39 and the host has glibc 2.35
- **THEN** `up` fails at layer `config` before any download, naming the recipe, the minimum and the host's version

#### Scenario: Recipe without this platform
- **WHEN** a recipe has no artifacts for the host's platform
- **THEN** `up` fails before any download and names the recipe and the platform

#### Scenario: User recipe without a checksum
- **WHEN** a user recipe declares an artifact without a checksum
- **THEN** `up` fails at layer `config` naming the file and the artifact

#### Scenario: Entrypoint escaping the object
- **WHEN** a user recipe's `interactive` `argv[0]` is `../../bin/sh`
- **THEN** it fails at layer `config` before any download, naming the recipe file and the entry

#### Scenario: Extracted entrypoint leaves the object
- **WHEN** the extracted path named by `argv[0]` is a symlink resolving outside the object
- **THEN** the install fails at layer `provider` naming the entry, and nothing is published

#### Scenario: Installed from the published crate
- **WHEN** devcroft is built from the unpacked `cargo package` output and `agent list` runs
- **THEN** it lists the embedded recipes

#### Scenario: Recipes follow XDG_CONFIG_HOME
- **WHEN** `XDG_CONFIG_HOME` points to an isolated directory holding a user recipe
- **THEN** that recipe is used and none from `~/.config/devcroft/agents/` is read

#### Scenario: Rollback
- **WHEN** the user sets `[agent] version` to the previous version of the recorded recipe and runs `up --recreate --agent-from-config`
- **THEN** the sandbox runs that version, from its object if still present

### Requirement: Agent materialization executes nothing downloaded
devcroft SHALL materialize recipe artifacts directly in Rust, verifying every
artifact's checksum before publication and installing only layouts implemented
without running downloaded code host-side (`single-binary` and `archive` in
this change; any unimplemented layout fails at layer `config` before download).
Extraction SHALL refuse absolute paths, parent-directory traversals, special
files, and symlinks or hardlinks that resolve outside the staging object. Every
fetch SHALL be bounded by a connect timeout, a stall timeout, a total timeout,
a maximum size and a maximum number of redirects, each of which SHALL stay on
HTTPS, and every extraction by a maximum entry count and expanded size.
Exceeding a limit SHALL fail at layer `provider` naming it, publish nothing,
remove staging and release the lifecycle lock.

#### Scenario: Checksum mismatch
- **WHEN** a downloaded artifact does not match its recipe checksum
- **THEN** the install fails at layer `provider`, nothing is published, and no partial object remains

#### Scenario: Sentinel package
- **WHEN** a pinned package's files would write a marker if executed during installation
- **THEN** after materialization the marker is absent from the host

#### Scenario: Archive escape
- **WHEN** an artifact contains an entry or a link that resolves outside the staging object
- **THEN** the install fails at layer `provider` naming the entry

#### Scenario: Stalled download
- **WHEN** the artifact server accepts the connection and then stops sending
- **THEN** `up` fails at layer `provider` naming the stall timeout, no staging remains, and `devcroft down` on that sandbox then proceeds without waiting

#### Scenario: Redirect off HTTPS
- **WHEN** the artifact URL redirects to an `http:` URL, or past the redirect limit
- **THEN** the install fails at layer `provider` naming the redirect, and nothing is published

#### Scenario: Expansion bomb
- **WHEN** an archive with a matching checksum expands beyond the expanded-size limit
- **THEN** the install fails at layer `provider` naming the limit and nothing is published

### Requirement: Agent objects are immutable, shared, leased and pinned on explicit install
An installed agent SHALL be published atomically as a read-only object
identified by its recipe, version and artifacts, shared by every sandbox that
selects it. `devcroft agent install <recipe>` SHALL record an explicit install
pin naming the object's id in the agent artifact store, and `devcroft agent uninstall <recipe>` SHALL
remove that pin and sweep unreferenced objects. Automatic sweeps during `up` or
`rm` SHALL remove an agent object only when neither any recorded sandbox `Meta`
nor any explicit install pin references it. Every path that publishes an agent
object (`up`, `agent install` and `agent run`) SHALL take the agent store's
lease before publishing and hold it until an install pin or a recorded `Meta`
references the object, whatever the project's provider. `rm` SHALL read the
sandbox's `Meta` and remove its state under that sandbox's lifecycle lock, and
SHALL sweep agent objects under the store's lease.

#### Scenario: Two sandboxes, one object
- **WHEN** two sandboxes select the same recipe version
- **THEN** both use one object and neither can write to it

#### Scenario: Explicit install survives unrelated sandbox removal
- **WHEN** the user runs `devcroft agent install opencode` before creating a sandbox for it, and then removes an unrelated sandbox with `devcroft rm`
- **THEN** the `opencode` object remains installed and `agent run` can use it without re-downloading

#### Scenario: User recipe sharing a name and version
- **WHEN** an embedded recipe version is installed and a user recipe with the same name and version but another checksum is then installed
- **THEN** both objects are pinned, and neither install removes the other's object

#### Scenario: Uninstall sweeps unreferenced object
- **WHEN** the user runs `devcroft agent uninstall opencode` and no sandbox `Meta` references that object
- **THEN** the install pin and the object are removed

#### Scenario: Last reference removed for an unpinned object
- **WHEN** an agent object was materialized implicitly by `up` without `agent install` and the last sandbox referencing it is removed
- **THEN** the object is swept

#### Scenario: Concurrent removal under a closure provider
- **WHEN** a nix project's `up` with an agent runs while another sandbox is removed
- **THEN** the agent object is not swept before this `up` records it

#### Scenario: Install concurrent with removal
- **WHEN** `agent install opencode` publishes an object while `rm` removes the last sandbox referencing another version and sweeps
- **THEN** afterwards the installed object and its pin both exist, and no pin names a missing object

#### Scenario: Remove racing a start
- **WHEN** `rm` and `up` run concurrently on the same sandbox
- **THEN** either `rm` removes the state `up` recorded, or `up` creates it afresh after `rm`; no agent object that a surviving `Meta` references is swept

### Requirement: Project commands keep their own runtime
The agent SHALL start inside the project's sandbox only through devcroft
(`agent run` in this change), by absolute paths into its object. Selecting an
agent SHALL add nothing to `PATH`, and the agent's command SHALL NOT resolve on
`PATH` in `devcroft shell`, `exec` or SSH sessions. Variables in the recipe's
`reserved_env` SHALL be refused in `manifest.env.vars` and `manifest.env.forward`
at layer `config`, and SHALL be absent from the agent's environment, including
when the provider or its activation script exports them. Names in
`requires.runtime_env` SHALL also be removed from the agent's environment, and
`up` SHALL print one line naming each one removed and saying that commands the
agent starts do not see it. Before connecting, `agent run` SHALL verify from
`Meta` that the running keeper supports variable unsetting, refusing an older
keeper and naming `up [--name <n>] --recreate`. `[launch.env]` SHALL reach the
agent's processes only, and SHALL NOT set `PATH`, `HOME`, `XDG_*`, `LD_*`,
`DYLD_*`, `requires.runtime_env`, `reserved_env` or the recipe's credential
names. The agent's executable headers SHALL be checked before teardown against
the agent's own grants and the baseline, never against the provider's grants.

#### Scenario: Divergent Node versions
- **WHEN** the project's environment provides Node 16 and the agent's recipe provides Node 24
- **THEN** the agent process runs on Node 24
- **AND** a `node --version` the agent runs through its shell tool reports Node 16

#### Scenario: Sentinel on the project's PATH
- **WHEN** the project's `PATH` holds a `node` that writes a marker when run
- **THEN** starting the agent and completing a task that edits a file writes no marker

#### Scenario: Launch variables stay with the agent
- **WHEN** a recipe sets `[launch.env]` and the user runs `devcroft exec -- env`
- **THEN** none of those variables is listed

#### Scenario: Project sets a runtime variable
- **WHEN** the project's environment sets `NODE_OPTIONS=--require /project/hook.js` and the recipe declares it in `requires.runtime_env`
- **THEN** the agent's process has no `NODE_OPTIONS` and loads no project code, `devcroft exec -- env` still lists it, and `up` prints one line naming it and the recipe

#### Scenario: No agent command on PATH
- **WHEN** a sandbox with an agent is up and `devcroft exec -- sh -c 'command -v opencode'` runs
- **THEN** the command is not found, and `devcroft agent run` still starts the agent

#### Scenario: Provider exports a reserved variable
- **WHEN** the provider or its activation script exports a variable reserved by a fixture recipe
- **THEN** it is absent from the agent launched by `agent run`

#### Scenario: Agent executable that needs the project's closure
- **WHEN** a fixture recipe's executable names an interpreter or library that only the project's provider grants
- **THEN** `up` fails at layer `config` naming that path, before any teardown

### Requirement: Agent state lives in HOME with symlink-safe scoped updates
The agent SHALL keep its config, history, cache and login in the sandbox's own
`HOME`, outside devcroft's data directory and its immutable package. Every
host-side read, merge or write inside `HOME` SHALL traverse directory
components via `O_NOFOLLOW` directory descriptors, refuse symlinked directories
or malformed/non-regular target files at layer `config` without following them,
and atomically update only managed entries via exclusive temp file plus
`renameat`. Unrelated settings and stored logins SHALL survive restart and
recreation. `Meta` SHALL persist `mode1_recipes` (every recipe name and its
relative `state_paths` ever run in shared mode on this sandbox) across `down`,
`up`, `up --recreate`, `--no-agent` and recipe switches until `rm`. Only `rm`
removes the sandbox HOME. When the sandbox runs without an agent or with a
recipe other than one recorded in `mode1_recipes`, `up` SHALL print one line
naming every recorded state path still present in `HOME` and saying that
project processes can read it.

#### Scenario: Agent writes history
- **WHEN** the agent records a session
- **THEN** the files appear under `<project>/.devcroft/<name>/home` and not in the object or `~/.local/share/devcroft`

#### Scenario: User settings in HOME survive restart
- **WHEN** the user or agent writes unrelated settings or a login token in the sandbox's `HOME` and runs `down` followed by `up`
- **THEN** devcroft's managed entry is present and the user's settings and token remain intact

#### Scenario: Symlink planted in HOME is refused without touching its target
- **WHEN** a process inside the sandbox replaces a managed directory or config file under `HOME` with a symlink to a host file and the user runs `up`
- **THEN** `up` fails at layer `config` naming the symlinked path and the target host file is unmodified

#### Scenario: Mode 1 recipe provenance survives opt-out
- **WHEN** a sandbox runs with an agent, is recreated with `--no-agent`, and `Meta` is inspected
- **THEN** `mode1_recipes` still records that recipe and its `state_paths` until `rm`

#### Scenario: Opting out does not hide retained state
- **WHEN** a sandbox whose agent stored a login under one of its `state_paths` is recreated with `--no-agent`
- **THEN** `up` names that path and says project processes can still read it

#### Scenario: why agrees with the kernel
- **WHEN** `devcroft why` is asked about a file the agent wrote in `HOME`
- **THEN** it reports the write as allowed

### Requirement: Deterministic authentication mode and brokered first recipe
For target recipe `R`, the authentication mode SHALL be the explicit
`--agent-auth` flag's, else `Meta`'s recorded mode only when `Meta` already
records recipe `R` (and `--agent-from-config` was not passed), else the user
configuration's `[agent] auth` when `[agent]` names `R`, else `R`'s
`default_auth` — never inherited from a different previously recorded recipe
and never chosen from ambient shell variables. The first maintained recipe
SHALL use one named provider and a qualified brokered `api-key` mode. Under
`api-key`, the key SHALL reach only the broker, `network.default = "allow"`
SHALL fail at layer `config` naming `--no-agent`, and the selected recipe SHALL
add only its declared destinations while disclosing that the sandbox shares its
broker route with project commands. A missing/empty key or an authentication
mode not qualified for this recipe and platform SHALL fail at layer `config`,
naming the remedy and supported modes for this recipe and platform.

#### Scenario: Broker on an empty allowlist
- **WHEN** the project has an empty allowlist and the user selects the qualified recipe with its key exported
- **THEN** the request reaches the named provider through devcroft’s broker and the project environment contains no real key

#### Scenario: Default allow refuses broker
- **WHEN** the project's `network.default` is `"allow"` and the mode is `api-key`
- **THEN** `up` fails at layer `config` naming the agent and `--no-agent`

### Requirement: One source for each of the recipe's credentials
In every mode, `up` SHALL refuse at layer `config` a `manifest.env.vars`,
`manifest.env.forward` or `[credentials] env` entry naming one of the recipe's
credential variables, and a user `[[credentials.broker]]` entry whose host is
one of the recipe's active hosts or whose variables are the recipe's, naming
both declarations. Under `api-key`, an entry identical to the recipe's broker
(same host, variables, endpoint variable and placeholder) SHALL be used once,
and `up` SHALL say so; one differing in any of these SHALL be refused. The
recipe's credential variables SHALL be removed from everything the agent
inherits (the provider's environment and the activation overlay) and the
selected mode's values applied last, and `up` SHALL print one line naming each
variable it removed. In `api-key` mode, the key SHALL be
read from a non-empty host environment variable (an unset or empty variable
fails at layer `config` unless the configured credential command supplies
it). Its value SHALL stay out of logs and metadata.

#### Scenario: Forwarded key under api-key
- **WHEN** `devcroft.toml` lists the recipe's credential name in `[env] forward` and the mode is `api-key`
- **THEN** `up` fails at layer `config` naming both declarations

#### Scenario: Provider exports the key variable
- **WHEN** the project's provider environment sets the recipe's key variable to a value and the mode is `api-key`
- **THEN** the agent receives only the placeholder and the broker endpoint, and `up` names the variable it removed

#### Scenario: Broker entry differing only in placeholder
- **WHEN** a user broker entry matches the recipe's host, variables and endpoint variable but names another placeholder
- **THEN** `up` fails at layer `config` naming both declarations

### Requirement: Interactive run
`devcroft agent run [<recipe>]` SHALL resolve its target sandbox by explicit
`--sandbox <name>` or by the deeper of the nearest manifest and recorded root
containing canonical `cwd` (refusing ambiguity at one root). When a Git
repository root (a `.git` directory or file, checked by name only) lies between
`cwd` and that root, it SHALL fail at layer `config` naming the outer root and
`--sandbox`, rather than attaching a nested repository to its parent's sandbox
implicitly. The same resolver SHALL serve `agent acp`, `status` and every
remedy. When `Meta`
already exists with `Unset`, `Disabled` or another recipe, it SHALL fail at
layer `config` naming `[cd <quoted project_root> && ]devcroft up [--name <n>]
--recreate [--agent <recipe>]`. When no `Meta` exists yet and neither
`<recipe>` nor `[agent]` selects a recipe, it SHALL fail at layer `config`
before downloading or starting `up`. Otherwise, when run interactively, it
SHALL materialize the target recipe's object if absent (from the recipe recorded
in `Meta` when one exists, even if this release no longer ships it), bring a
down sandbox up
with the user configuration (passing `--agent <recipe>` when no `Meta` exists
yet and `<recipe>` was passed on the command line), and run the recipe's
`interactive` `argv` on a PTY sized to the local terminal.

#### Scenario: Cold interactive start without global agent config
- **WHEN** `~/.config/devcroft/config.toml` has no `[agent]` and the user runs `devcroft agent run <recipe>` in a terminal on a brand-new project whose agent object is not yet installed
- **THEN** devcroft materializes the object with progress on `stderr`, brings the sandbox up with `<recipe>` recorded in `Meta` (including its grants, hosts and broker), and attaches the agent's TUI on a PTY of the terminal's size

#### Scenario: No recipe selected anywhere
- **WHEN** `devcroft agent run` is invoked without `<recipe>` on a brand-new project and `[agent]` is unset
- **THEN** it fails at layer `config` before resolving the provider or creating state

#### Scenario: Nested repository without its own manifest
- **WHEN** `/repo` has a manifest and a running sandbox, and `devcroft agent run` runs in `/repo/vendor/lib`, a separate Git repository with no `devcroft.toml`
- **THEN** it fails at layer `config` naming `/repo` and `--sandbox`, and attaches nothing

#### Scenario: OSC 7501 through agent run
- **WHEN** an agent started by `agent run` emits a capability query and the terminal replies without a newline
- **THEN** `agent run` meets the `exec` capability's OSC 7501 requirement, as `devcroft shell` does

#### Scenario: First project task
- **WHEN** the qualified agent is asked to edit a source file and run a project test
- **THEN** the edit is visible in the workspace, the test uses the project's toolchain, and devcroft transports any OSC 7501 sequences unchanged without fabricating identity or state when the harness emits none

### Requirement: Platform-scoped recipe requirements are checked at up
`up` SHALL merge `[requires]` with `[requires.<current-platform>]` and check
them before restriction. A missing `procfs` requirement on the current platform
SHALL refuse the recipe at layer `backend`; a Linux-only `procfs` requirement
SHALL NOT refuse macOS. A missing declared command on the sandbox's `PATH`
SHALL produce one warning naming it. A recipe requiring `loopback_any` SHALL be
refused at layer `config` unless the project manifest sets
`network.loopback = "any"`.

#### Scenario: Bun harness on a Linux host without procfs
- **WHEN** a recipe declaring `procfs = true` for Linux is selected on a Linux host where the sandbox has no private procfs
- **THEN** `up` fails at layer `backend` naming the recipe and `procfs`

#### Scenario: Linux procfs requirement does not refuse macOS
- **WHEN** the same recipe is selected on `macos-arm64`, where `[requires.macos-arm64]` does not require `procfs`
- **THEN** `up` does not fail on `procfs`

### Requirement: Advertised support comes from a complete qualification record
Each recipe version SHALL ship with a versioned qualification record. The
record SHALL have one row per artifact digest, devcroft version, platform,
launch path (`agent run`, `agent acp`) and auth mode. For every mandatory
probe, a row SHALL record what the recipe declares and what the probe
observed, as `pass`, `fail`, `drift` (declared and observed disagree),
`skipped` or `unknown` (no probe exists). A row SHALL count as qualified only
when every mandatory probe ran and passed. A skipped or unknown mandatory probe
SHALL leave the row unqualified, never green. `agent list`, `doctor`, the CI
release gate and `docs/agents.md` SHALL read the same record.

#### Scenario: A mandatory probe skipped
- **WHEN** the broker probe for `macos-arm64` was skipped in the qualifying run
- **THEN** `agent list` shows that platform as unqualified, naming the skipped probe, and `up` refuses the recipe there

#### Scenario: Declared but not observed
- **WHEN** the recipe declares native OSC 7501 emission and the probe observed none
- **THEN** the row records `drift`, and the capability is not advertised

## ADDED Requirements

### Requirement: Agent recipes are owned by devcroft
devcroft SHALL ship agent recipes as data in its release, reproducible from
committed repository inputs without requiring `mise`, `npm` or `nix` on the
host at runtime. Each recipe version SHALL pin, per supported platform, every
artifact it installs with a URL and a checksum, its ACP support mode (`native`,
`adapter` or `none`), each component's version, its entrypoints, any `reserved_env` names stripped on
launch, its network destinations, its supported authentication modes and its
platform-scoped sandbox requirements. A user recipe in the user configuration
directory SHALL pass the same validation, with a checksum on every artifact,
and SHALL be marked as a user recipe wherever the recipe is shown. An unknown
recipe or version, or an unsupported platform, SHALL fail at layer `config`,
exit code 2, listing what is available.

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
devcroft SHALL materialize recipe artifacts directly in Rust, verifying every
artifact's checksum before publication and installing by sanitized archive
extraction or script-free lockfile layout without running any downloaded code
host-side. Extraction SHALL refuse absolute paths, parent-directory traversals,
special files, and symlinks or hardlinks that resolve outside the staging
object. A lock entry that declares an install script SHALL refuse the recipe
unless the recipe explicitly marks that script unnecessary from recorded
qualification evidence. A recipe that requires code execution at install SHALL
run it only inside confined provisioning, writing only into the staging object.

#### Scenario: Checksum mismatch
- **WHEN** a downloaded artifact does not match its recipe checksum
- **THEN** the install fails at layer `provider`, nothing is published, and no partial object remains

#### Scenario: Sentinel package
- **WHEN** a pinned package's files would write a marker if executed during installation
- **THEN** after materialization the marker is absent from the host

#### Scenario: Archive escape
- **WHEN** an artifact contains an entry or a link that resolves outside the staging object
- **THEN** the install fails at layer `provider` naming the entry

### Requirement: Agent objects are immutable, shared, leased and pinned on explicit install
An installed agent SHALL be published atomically as a read-only object
identified by its recipe, version and artifacts, shared by every sandbox that
selects it. `devcroft agent install <recipe>` SHALL record an explicit install
pin naming the object's id in the agent artifact store, and `devcroft agent uninstall <recipe>` SHALL
remove that pin and sweep unreferenced objects. Automatic sweeps during `up` or
`rm` SHALL remove an agent object only when neither any recorded sandbox `Meta`
nor any explicit install pin references it. `up` SHALL hold the agent store's
lease whenever an agent is selected, whatever the project's provider.

#### Scenario: Two sandboxes, one object
- **WHEN** two sandboxes select the same recipe version
- **THEN** both use one object and neither can write to it

#### Scenario: Explicit install survives unrelated sandbox removal
- **WHEN** the user runs `devcroft agent install claude-code` before creating a sandbox for it, and then removes an unrelated sandbox with `devcroft rm`
- **THEN** the `claude-code` object remains installed and `agent acp` can use it without re-downloading

#### Scenario: User recipe sharing a name and version
- **WHEN** an embedded recipe version is installed and a user recipe with the same name and version but another checksum is then installed
- **THEN** both objects are pinned, and neither install removes the other's object

#### Scenario: Uninstall sweeps unreferenced object
- **WHEN** the user runs `devcroft agent uninstall claude-code` and no sandbox `Meta` references that object
- **THEN** the install pin and the object are removed

#### Scenario: Last reference removed for an unpinned object
- **WHEN** an agent object was materialized implicitly by `up` without `agent install` and the last sandbox referencing it is removed
- **THEN** the object is swept

#### Scenario: Concurrent removal under a closure provider
- **WHEN** a nix project's `up` with an agent runs while another sandbox is removed
- **THEN** the agent object is not swept before this `up` records it

### Requirement: The agent's runtime does not reach the project's commands and strips reserved overrides
The agent SHALL start inside the project's sandbox through absolute paths into
its object, and nothing from its object SHALL be prepended to `PATH`. Only the
launcher directory, in the sandbox's artifact directory (read-only from inside)
and holding only the agent's own command, SHALL be on `PATH`, after the
project's entries and the tooling layer. The launcher SHALL be neither a shell
script nor a symlink to the harness, since it must change the environment.
Variables in the recipe's `reserved_env` (such as `CLAUDE_CODE_EXECUTABLE`)
SHALL be refused in `manifest.env.vars` and `manifest.env.forward` at layer
`config`, and SHALL be absent from the agent's environment however it is
started (`agent acp`, `agent run` or the launcher), including when the provider
or its activation script exports them. Before connecting, `agent acp` and `agent
run` SHALL verify from `Meta` that the running keeper supports variable
unsetting, refusing an older keeper and naming `up [--name <n>] --recreate`.
The `devcroft` executable path written into the launcher shebang SHALL be
verified free of whitespace/newlines and within the kernel shebang limit before
restriction. `[launch.env]` SHALL reach the agent's processes only, and SHALL
NOT set `PATH`, `HOME`, `XDG_*`, `LD_*`, `DYLD_*`, `requires.runtime_env` or
`reserved_env` names.

#### Scenario: Divergent Node versions
- **WHEN** the project's environment provides Node 16 and the agent's recipe provides Node 24
- **THEN** the agent process runs on Node 24
- **AND** a `node --version` the agent runs through its shell tool reports Node 16

#### Scenario: Sentinel on the project's PATH
- **WHEN** the project's `PATH` holds a `node` that writes a marker when run
- **THEN** starting the agent and completing a task that edits a file writes no marker

#### Scenario: Inherited CLAUDE_CODE_EXECUTABLE cannot hijack the adapter
- **WHEN** the provider's environment exports `CLAUDE_CODE_EXECUTABLE` pointing to a marker-writing executable and `devcroft agent acp claude-code` runs
- **THEN** `CLAUDE_CODE_EXECUTABLE` is stripped before the adapter starts, the marker executable is not run, and no MCP headers appear in child `argv`

#### Scenario: Activation script exports a reserved variable
- **WHEN** the provider's activation script exports `CLAUDE_CODE_EXECUTABLE` and the agent is started by `agent acp`, by `agent run` and by its command in `devcroft shell`
- **THEN** the variable is absent from the agent's environment in all three

#### Scenario: Launch variables stay with the agent
- **WHEN** a recipe sets `[launch.env]` and the user runs `devcroft exec -- env`
- **THEN** none of those variables is listed

#### Scenario: Project sets a runtime variable
- **WHEN** the project's environment sets `NODE_OPTIONS` and the recipe declares it in `requires.runtime_env`
- **THEN** `up` prints one warning naming `NODE_OPTIONS` and the recipe

#### Scenario: Launcher on macOS
- **WHEN** the user runs the agent's command in `devcroft shell` on macOS
- **THEN** it starts, and no shell outside the sandbox's grants is executed

### Requirement: Agent state lives in HOME with symlink-safe scoped updates
The agent SHALL keep its config, history, cache and login in the sandbox's own
`HOME`, never under devcroft's data directory and never in its object. Every
host-side read, merge or write by devcroft inside `HOME` SHALL traverse
directory components via `O_NOFOLLOW` directory descriptors, refuse symlinked
directories or non-regular target files at layer `config` without following
them, merge only devcroft's managed keys so user preferences and stored logins
survive `down`, `up` and `up --recreate`, and commit via exclusive temp file
plus `renameat`. `HOME` SHALL be removed only at `rm`.

#### Scenario: Agent writes history
- **WHEN** the agent records a session
- **THEN** the files appear under `<project>/.devcroft/<name>/home` and not in the object or `~/.local/share/devcroft`

#### Scenario: User settings in HOME survive restart
- **WHEN** the user or agent writes unrelated settings or a login token in the sandbox's `HOME` and runs `down` followed by `up`
- **THEN** devcroft's managed hook entry is present and the user's settings and token remain intact

#### Scenario: Symlink planted in HOME is refused without touching its target
- **WHEN** a process inside the sandbox replaces a managed directory or config file under `HOME` with a symlink to a host file and the user runs `up`
- **THEN** `up` fails at layer `config` naming the symlinked path and the target host file is unmodified

#### Scenario: why agrees with the kernel
- **WHEN** `devcroft why` is asked about a file the agent wrote in `HOME`
- **THEN** it reports the write as allowed

### Requirement: One authentication mode per sandbox, chosen deterministically
A recipe SHALL declare one or more authentication modes (`api-key`, `login`,
`env`, optionally scoped per platform) and a default among them. For target
recipe `R`, the mode SHALL be the explicit `--agent-auth` flag's, else `Meta`'s
recorded mode only when `Meta` already records recipe `R` (and
`--agent-from-config` was not passed), else the user configuration's `[agent]
auth` when `[agent]` names `R`, else `R`'s default — never inherited from a
different previously recorded recipe. The contents of the shell SHALL NOT choose
the mode. When the mode's prerequisite is missing (or unsupported on this
platform), `up` SHALL fail at layer `config` naming it and the other modes the
recipe supports on this platform.

Under `api-key` the recipe's broker SHALL be active only in sandboxes selecting
that recipe and mode, and `network.default = "allow"` SHALL fail at layer
`config` naming `--no-agent`. Under `login` no broker and no key SHALL be
required, login hosts SHALL join the egress allowlist, and `up` SHALL announce
that the token in `HOME` is readable by the project. Under `env`, `up` SHALL
print the delivered-credential custody disclosure.

#### Scenario: Brokered API key on an empty allowlist
- **WHEN** a project with `network.allow = []` is brought up with `[agent] name = "claude-code"` under `api-key` and `ANTHROPIC_API_KEY` exported
- **THEN** `up` succeeds and the agent's request reaches the API through the broker

#### Scenario: Subscription login without an API key
- **WHEN** `ANTHROPIC_API_KEY` is unset and the user runs `up` with `[agent] name = "claude-code"` and `auth = "login"`
- **THEN** `up` succeeds without a broker, allows the recipe's API and login hosts, and announces `HOME` token visibility

#### Scenario: Exported key does not change the mode
- **WHEN** a sandbox recorded with `auth = "login"` is brought up from a shell that exports `ANTHROPIC_API_KEY`
- **THEN** it stays in `login` mode and the sandbox does not receive the key

#### Scenario: Missing key names the alternatives
- **WHEN** the mode is `api-key`, the key variable is unset and no credential command is configured
- **THEN** `up` fails at layer `config` naming `ANTHROPIC_API_KEY`, `[agent.credentials.claude-code]` and `--agent-auth login`

#### Scenario: Default allow refuses broker
- **WHEN** the project's `network.default` is `"allow"` and the mode is `api-key`
- **THEN** `up` fails at layer `config` naming the agent and `--no-agent`

### Requirement: One source for each of the recipe's credentials
In every mode, `up` SHALL refuse at layer `config` a `manifest.env.vars`,
`manifest.env.forward` or `[credentials] env` entry naming one of the recipe's
credential variables, and a user `[[credentials.broker]]` entry whose host is
one of the recipe's active hosts or whose variables are the recipe's, naming
both declarations. An entry identical to the recipe's broker under `api-key`
SHALL be used once, and `up` SHALL say so. A key SHALL be read from the host
variable when non-empty, else from the user configuration's command for that
recipe, run host-side as an `argv` without a shell and under a deadline. The
value SHALL NOT be recorded or logged, and under `api-key` SHALL reach the
broker only.

#### Scenario: Forwarded key under login
- **WHEN** `devcroft.toml` lists `ANTHROPIC_API_KEY` in `[env] forward` and the mode is `login`
- **THEN** `up` fails at layer `config` naming `[env] forward` and the recipe, rather than letting the API key replace the subscription

#### Scenario: Forwarded key under api-key
- **WHEN** `devcroft.toml` lists `ANTHROPIC_API_KEY` in `[env] forward` and the mode is `api-key`
- **THEN** `up` fails at layer `config` naming both declarations

#### Scenario: User broker for the same host under login
- **WHEN** the user configuration brokers `api.anthropic.com` and the mode is `login`
- **THEN** `up` fails at layer `config` naming the broker entry, since the brokered host would not be reachable directly

#### Scenario: Key command from a GUI editor
- **WHEN** an editor started by the OS, with no key in its environment, runs `agent acp` on a down sandbox whose recipe has a configured credential command
- **THEN** the sandbox comes up and the broker uses the command's value, which appears in no log, policy or `Meta`

#### Scenario: Project declares a credential command
- **WHEN** `devcroft.toml` contains `[agent.credentials.claude-code]`
- **THEN** loading it fails at layer `config` naming the user configuration file

### Requirement: ACP endpoint discovery, bounded startup and valid remedies
`devcroft agent acp` SHALL run on the host and resolve its target sandbox by
`--sandbox <name>`, or else by comparing both the nearest upward `devcroft.toml`
root and the innermost recorded `Meta` `project_root` containing canonical
`cwd` and choosing the deeper (more specific) root. Ambiguous sandboxes sharing
the same `project_root` SHALL fail at layer `config` naming each candidate and
`--sandbox`. `agent acp` SHALL refuse `acp = "none"` naming `devcroft agent run
<recipe>`, SHALL fail immediately on `stderr` naming `devcroft up [--name <n>]
--recreate [--agent <recipe>]` whenever `Meta` already exists on disk (running
or stopped) with `Unset`, `Disabled` or a recipe other than the target one, and
SHALL fail immediately on `stderr` without downloading or executing any
credential command when the agent object or a preflight prerequisite is missing.
When a brand-new sandbox (no `Meta`) or a stopped sandbox already recording the
target recipe is down, `agent acp` SHALL start `up` as a separate process that
owns its own completion and wait for it under a bounded deadline. At the
deadline, or when the client ends, that `up` SHALL be left to finish; `agent
acp` SHALL exit non-zero on `stderr` naming the log and the exact preparation
command. While a lifecycle operation is in progress it SHALL wait under the same
deadline and SHALL NOT start another. If an earlier background `up` failed, the
next `agent acp` SHALL report that earlier error on `stderr`, clear the failure
marker and start a fresh `up` under the deadline. A stale recipe on a running
sandbox SHALL be used with one `stderr` line naming the recreate command. It
SHALL pass `stdin` and `stdout` through unmodified (`pty: None`) with zero
devcroft messages or OSC sequences on `stdout`, including while `up` runs.

#### Scenario: Editor handshake on a first-time checkout
- **WHEN** a recipe object is installed, the project environment activates within the startup deadline, and a GUI-launched ACP client runs `devcroft agent acp` in a project where `up` has never run
- **THEN** `agent acp` brings the sandbox up with the user configuration and returns the agent's `initialize` response with no devcroft output on `stdout`

#### Scenario: Nested subproject does not attach to parent sandbox
- **WHEN** parent `/repo` is already up with recorded `Meta`, and an editor runs `devcroft agent acp` inside `/repo/sub` which has its own `devcroft.toml` and no recorded `Meta` yet
- **THEN** `agent acp` targets `/repo/sub`'s manifest rather than attaching to `/repo`'s running sandbox

#### Scenario: Cold provisioning outlives the deadline
- **WHEN** `agent acp` auto-starts a down sandbox whose provisioning exceeds the deadline
- **THEN** it exits non-zero on `stderr` naming the log and `devcroft up` for that project root, and the `up` continues to completion

#### Scenario: Editor retries during provisioning
- **WHEN** the editor runs `agent acp` again while that `up` is in progress
- **THEN** no second `up` starts, and a retry after it completes returns the `initialize` response

#### Scenario: Background up failed and next retry recovers
- **WHEN** a background `up` failed, the user fixes the cause, and `agent acp` runs again
- **THEN** `stderr` reports the earlier failure summary, clears the marker, starts a fresh `up`, and returns the `initialize` response once ready

#### Scenario: Stopped sandbox that never had a selection is not widened by agent acp
- **WHEN** a stopped sandbox on disk has `Meta` with no recorded agent selection (`Unset`) and `agent acp` (with or without `<recipe>`) runs
- **THEN** `agent acp` fails immediately without starting `up`, naming `devcroft up --recreate` (plus `--agent <recipe>` when one was passed)

#### Scenario: Different recipe on a named sandbox prints a valid command
- **WHEN** a sandbox brought up at `/repo` with `devcroft up --name wt2` runs another recipe, or none
- **THEN** `agent acp <recipe> --sandbox wt2` fails naming `devcroft up --name wt2 --recreate --agent <recipe>` (rooted at `/repo`)

#### Scenario: Upgrade does not break a running editor setup
- **WHEN** devcroft is upgraded to a release whose recipe differs and `agent acp` runs on a sandbox still up with the old one
- **THEN** it returns the `initialize` response and prints one staleness line on `stderr`

#### Scenario: Object not installed
- **WHEN** `agent acp` runs and the recipe's object is absent
- **THEN** it fails at once on `stderr`, naming `devcroft agent install <recipe>`

#### Scenario: Editor started outside any project
- **WHEN** `agent acp` runs with `cwd = /` and no `--sandbox`
- **THEN** it fails saying no sandbox or `devcroft.toml` contains the directory and names `--sandbox`

### Requirement: Interactive run
`devcroft agent run` SHALL resolve its target sandbox using the same
innermost-root discovery order as `agent acp`, fail naming `devcroft up [--name
<n>] --recreate [--agent <recipe>]` if `Meta` already exists with `Unset`,
`Disabled` or another recipe, materialize the recipe's object if absent when run
interactively, bring a brand-new or matching stopped sandbox up with the user
configuration if down, and run the recipe's `interactive` `argv` on a PTY sized
to the local terminal.

#### Scenario: Cold interactive start
- **WHEN** the user runs `devcroft agent run <recipe>` in a terminal on a project whose agent object is not yet installed
- **THEN** devcroft materializes the object with progress on `stderr`, starts the sandbox, and attaches the agent's TUI on a PTY of the terminal's size

#### Scenario: OSC 7501 through agent run
- **WHEN** an agent started by `agent run` emits a capability query and the terminal replies without a newline
- **THEN** `agent run` meets the `exec` capability's OSC 7501 requirement, as `devcroft shell` does

### Requirement: ACP session lifecycle
When the ACP client closes `stdin`, devcroft SHALL forward it and give the agent
a grace period to exit. After that period, or when the client process dies, the
session's process group SHALL be ended. The sandbox, its services and every
other session SHALL stay up. devcroft SHALL NOT interpret ACP cancellation.

#### Scenario: Editor closes
- **WHEN** the editor exits while the agent is running a command
- **THEN** after the grace period no process of that session remains, and the sandbox and its services are still up

#### Scenario: Two editors
- **WHEN** two clients run `agent acp` on one sandbox and one disconnects
- **THEN** the other session continues

#### Scenario: Request cancelled
- **WHEN** the client cancels a request without closing the session
- **THEN** the message reaches the agent unmodified and the session continues

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

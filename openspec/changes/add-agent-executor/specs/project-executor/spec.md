## ADDED Requirements

### Requirement: Project operations run in a collision-free project snapshot
In mode 2, `up` SHALL fail at layer `config` before teardown or provisioning if
`manifest.env.vars`, `manifest.env.forward` or user `[credentials] env` names
any credential or launch variable of the selected recipe (broker `env` or
`endpoint_env`, `auth = "env"` variable, `[launch.env]` or `reserved_env`).
Every spawn requested through the project executor SHALL run with an
environment built from nothing but the project snapshot captured at `up` and
the declared deltas, with `cwd` resolved inside the pair's workspace root under
the project's policy. The executor SHALL NOT accept a root, mount, grant,
network policy or environment snapshot from the caller.

#### Scenario: Direct argv without a shell
- **WHEN** a qualified harness's exec tool runs `node -p process.versions.node` without a shell
- **THEN** it reports the project's Node version

#### Scenario: Descendants and shebangs
- **WHEN** `npm test` runs a `#!/usr/bin/env node` script that starts another `node`
- **THEN** every generation runs the project's Node with the project's environment

#### Scenario: Forwarded agent key collision is refused before snapshot capture
- **WHEN** `devcroft.toml` lists the recipe's broker key or `auth = "env"` key in `[env] forward` or `[env.vars]`
- **THEN** `up` fails at layer `config` before provisioning rather than copying the key into the project snapshot

#### Scenario: Agent markers and auth variables do not cross
- **WHEN** the agent's environment holds `AGENT_ONLY`, `NODE_OPTIONS`, a delivered `auth = "env"` key and a different `PATH`
- **THEN** none of them is visible to a process the executor starts

#### Scenario: Absolute path into the agent's object
- **WHEN** the executor is asked to run the agent's private `node` by absolute path
- **THEN** the spawn fails, because that path is not in the project's view, and nothing is substituted

### Requirement: Mode 2 is opt-in and refused when not eligible
A sandbox SHALL run in mode 2 only when the recorded selection asks for it. `up`
SHALL fail before teardown, naming every unmet condition, unless
`network.default` is `"deny"`, the host blocks each tree from connecting to the
other's unix sockets and proxy port (on Linux a constructed mount view and
network namespace, on macOS the verified Seatbelt socket and `proxy_only`
denials), the recipe version is qualified, and — whenever the selected auth mode
places a real upstream credential in the agent's environment (`auth = "env"`) —
each tree's process environment is unreadable from the other. It SHALL NOT fall
back to mode 1.

#### Scenario: Allow-default project
- **WHEN** a project sets `network.default = "allow"` and mode 2 is requested
- **THEN** `up` fails at layer `config` naming it, and no tree is started

#### Scenario: Host without a mount view
- **WHEN** mode 2 is requested on a Linux host where the mount view cannot be constructed, with or without a Landlock-only fallback
- **THEN** `up` fails at layer `backend` naming it

#### Scenario: Delivered key where environments are readable
- **WHEN** mode 2 is requested with `auth = "env"` on a host where one tree can read the other's process environment
- **THEN** `up` fails naming the condition

#### Scenario: Upgrade
- **WHEN** a mode 1 sandbox is recreated by a devcroft whose recipe is qualified for mode 2
- **THEN** it stays in mode 1

### Requirement: Executor socket exclusivity
The executor SHALL be reachable only through a pathname socket created before
restriction inside a `0700` agent-sockets directory visible/connectable to its
paired agent tree alone (via mount view on Linux and verified Seatbelt
`network-outbound` + trailing path denials on macOS), accepting independent
connections and refusing any `cwd` outside its paired workspace root.

#### Scenario: Escaping cwd
- **WHEN** a request names `cwd = "../other"` or an absolute host path
- **THEN** the request is refused and no process starts

#### Scenario: MCP server deep in the agent's tree
- **WHEN** an MCP server started by the harness, several generations below the agent, connects to the executor while another session is running
- **THEN** both connections are served

#### Scenario: Project tree cannot reach the executor on Linux or macOS
- **WHEN** a project process in a mode 2 sandbox tries to `connect()` to the executor's socket
- **THEN** the connection is refused by the OS boundary on both Linux and macOS

### Requirement: Harness routing is qualified per recipe version and merged safely
A recipe SHALL be offered in mode 2 only for versions where devcroft disables
the harness's native exec tool and supplies the executor-backed one, as
measured, through `O_NOFOLLOW` descriptor-anchored, key-scoped configuration in
the agent's `HOME` that the editor's ACP session parameters cannot override.
devcroft SHALL NOT parse or rewrite ACP to achieve it. Routing configuration
SHALL contribute to `recipe_digest` for staleness detection. Other versions
SHALL be refused in mode 2, and `status` SHALL say which mode is active.
Qualification SHALL record the workspace files the harness runs or obeys, and a
recipe SHALL use a lever that disables them where the harness has one.

#### Scenario: Native exec cannot be disabled
- **WHEN** a recipe version's native exec tool cannot be turned off
- **THEN** requesting mode 2 fails at layer `config` naming the recipe version and the reason

#### Scenario: Editor re-enables the native tool
- **WHEN** the editor's `session/new` asks for the native exec tool on a recipe version that devcroft cannot keep disabled
- **THEN** that version is not qualified for mode 2

#### Scenario: Project-level harness configuration
- **WHEN** a project command writes a project-level hook for the harness and the recipe version has a qualified lever against it
- **THEN** the hook does not run in the agent tree

### Requirement: Signals, exit and stdio are faithful
Executor sessions SHALL preserve exit codes and signals, deliver cancel to the
process group, keep `stdout` and `stderr` separate in piped mode with no
injected OSC sequences, and leave no process behind after `down` or after the
agent exits. The RPC SHALL support a PTY with resize; a recipe SHALL claim PTY
sessions only through a transport that carries one, never through MCP.

#### Scenario: Cancel reaches the group
- **WHEN** the agent cancels a running test whose runner started children
- **THEN** the whole group terminates and the exit status reports the signal

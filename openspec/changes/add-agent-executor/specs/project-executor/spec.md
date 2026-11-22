## ADDED Requirements

### Requirement: Project operations run in the project snapshot
In mode 2, every spawn requested through the project executor SHALL run with
an environment built from nothing but the project snapshot captured at `up`
and the declared deltas. It SHALL run with `cwd` resolved inside the pair's
workspace root, under the project's policy. The executor SHALL NOT accept a
root, mount, grant, network policy or environment snapshot from the caller.

#### Scenario: Direct argv without a shell
- **WHEN** a qualified harness's exec tool runs `node -p process.versions.node` without a shell
- **THEN** it reports the project's Node version

#### Scenario: Descendants and shebangs
- **WHEN** `npm test` runs a `#!/usr/bin/env node` script that starts another `node`
- **THEN** every generation runs the project's Node with the project's environment

#### Scenario: Agent markers do not cross
- **WHEN** the agent's environment holds `AGENT_ONLY`, `NODE_OPTIONS` and a different `PATH`
- **THEN** none of them is visible to a process the executor starts

#### Scenario: Absolute path into the agent's object
- **WHEN** the executor is asked to run the agent's private `node` by absolute path
- **THEN** the spawn fails, because that path is not in the project's view, and nothing is substituted

### Requirement: The executor is bound to one pair
The executor SHALL be reachable only through a pathname socket created before
restriction and visible to its paired agent tree alone, accepting independent
connections. It SHALL serve one workspace root, and a `cwd` outside that root
SHALL be refused.

#### Scenario: Escaping cwd
- **WHEN** a request names `cwd = "../other"` or an absolute host path
- **THEN** the request is refused and no process starts

#### Scenario: MCP server deep in the agent's tree
- **WHEN** an MCP server started by the harness, several generations below the agent, connects to the executor while another session is running
- **THEN** both connections are served

#### Scenario: Project tree cannot reach the executor
- **WHEN** a project process tries to connect to the executor's socket
- **THEN** the connection fails

### Requirement: Harness routing is qualified per recipe version
A recipe SHALL be offered in mode 2 only for versions where devcroft disables
the harness's native exec tool and supplies the executor-backed one, as
measured, through configuration that the editor's ACP session parameters
cannot override. devcroft SHALL NOT parse or rewrite ACP to achieve it. Other
versions SHALL run in mode 1, and `status` SHALL say which mode is active.

#### Scenario: Native exec cannot be disabled
- **WHEN** a recipe version's native exec tool cannot be turned off
- **THEN** the agent runs in mode 1 and `status` names mode 1 and the reason

#### Scenario: Editor re-enables the native tool
- **WHEN** the editor's `session/new` asks for the native exec tool on a recipe version that devcroft cannot keep disabled
- **THEN** that version is not qualified for mode 2

### Requirement: Signals, exit and stdio are faithful
Executor sessions SHALL preserve exit codes and signals, deliver cancel to the
process group, keep stdout and stderr separate in piped mode, and leave no
process behind after `down` or after the agent exits. The RPC SHALL support a
PTY with resize; a recipe SHALL claim PTY sessions only through a transport
that carries one, never through MCP.

#### Scenario: Cancel reaches the group
- **WHEN** the agent cancels a running test whose runner started children
- **THEN** the whole group terminates and the exit status reports the signal

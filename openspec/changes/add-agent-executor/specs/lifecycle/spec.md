## ADDED Requirements

### Requirement: Two restricted trees under an unrestricted parent
For a sandbox in mode 2, `up` SHALL start an unrestricted host-side parent.
The parent SHALL create every listener and fd before restriction, then start
the project executor and the agent as separate children, each applying its
own profile to itself. The parent SHALL run no session itself. Every process
of both trees and both proxies SHALL be recorded as it starts. Sandboxes
without an agent, or in mode 1, SHALL keep the single self-restricting keeper
unchanged.

#### Scenario: Agent-less sandbox unchanged
- **WHEN** a sandbox without an agent is brought up
- **THEN** its process topology and compiled profile are identical to those before this change

#### Scenario: Down tears down both trees
- **WHEN** `down` runs on a mode 2 sandbox
- **THEN** the parent, the executor, the agent, both proxies and every descendant have exited

#### Scenario: Parent killed on macOS
- **WHEN** the parent receives `SIGKILL` on macOS and `down` runs
- **THEN** no process of either tree or either proxy is left running

### Requirement: Separate homes and explicit sandbox identity
In mode 2 the agent tree's `HOME` SHALL be outside the shared workspace and
outside devcroft's data directory, granted to the agent profile only and
removed at `rm`. Each tree SHALL receive its sandbox directory explicitly, so
the in-sandbox `agent` verbs and the denial hook work from both trees.

#### Scenario: Project reads the agent's home
- **WHEN** a project command tries to read a file the agent wrote into its `HOME`
- **THEN** the read is refused

#### Scenario: Agent verbs from the agent tree
- **WHEN** the agent runs `devcroft agent progress <message>`
- **THEN** the report is recorded for the sandbox

### Requirement: Commands reach the right tree
`exec`, `shell`, `ssh` and services SHALL reach the project tree. `agent acp`
and `agent run` SHALL reach the agent tree through its own control socket.
`status`, `logs`, `policy --render` and `why` SHALL report both trees,
labelled.

#### Scenario: Agent launcher in the project's shell
- **WHEN** the user types the agent's command in `devcroft shell` on a mode 2 sandbox
- **THEN** the command is not found and `devcroft shell` has said to use `devcroft agent run`

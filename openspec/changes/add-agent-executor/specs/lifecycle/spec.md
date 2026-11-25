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

### Requirement: Separate per-recipe homes and Mode 1 state quarantine
In mode 2 the agent tree's `HOME` SHALL live under
`~/.local/share/devcroft-agent-homes/<sandbox-id>/<recipe>/home` outside the
shared workspace and outside devcroft's data directory, granted to that recipe's
agent profile only, updated via `O_NOFOLLOW` descriptor-anchored merges so user
settings and Mode 2 `auth = "login"` tokens survive `down` and `up` without
being shared across different recipes, and removed at `rm`. Whenever mode 2 is
requested for a recipe `R` recorded in `Meta.mode1_recipes` (having ever run in
mode 1 on that sandbox, including before an intermediate `--no-agent` or recipe
switch) and any of `R`'s `state_paths` exists in `<project>/.devcroft/<name>/home`,
`up` SHALL fail at layer `config` before teardown, listing them and naming
`--quarantine-agent-state`. With that flag `up` SHALL move them without
following symlinks to a directory that no profile grants, clear `R` from
`mode1_recipes`, SHALL NOT delete or copy them into the agent's `HOME`, and
SHALL print their new location and that a login used in mode 1 should be
revoked. A failed move SHALL fail `up` and leave the source in place. When `R`
is not in `mode1_recipes`, `up` SHALL NOT touch `state_paths` in the project's
`HOME`. Each tree SHALL receive its sandbox identity explicitly so in-sandbox
`agent` verbs and the denial hook work from both trees.

#### Scenario: Project and other recipes cannot read the agent's home
- **WHEN** an agent writes a login token into its Mode 2 `HOME`, a project command tries to read it, or the sandbox is recreated in Mode 2 with a different recipe
- **THEN** the project read is refused and the new recipe gets its own empty `HOME` without access to the previous recipe's `HOME`

#### Scenario: Transition without the flag, direct or after opting out
- **WHEN** a mode 1 sandbox holds a login token under a `state_path` for `claude-code` and runs `up --agent-mode 2 --recreate` (either directly, or after `up --no-agent --recreate` followed by `up --agent claude-code --agent-mode 2 --recreate`)
- **THEN** it fails naming the paths and `--quarantine-agent-state`, no Mode 2 tree starts, and no file changed

#### Scenario: Transition with the flag
- **WHEN** the same command runs with `--quarantine-agent-state`
- **THEN** every file is in the quarantine directory, neither tree can read it, the agent's `HOME` has no copy, and `up` names the directory and the revocation

#### Scenario: Interrupted move
- **WHEN** moving the state fails part-way
- **THEN** `up` fails and no file has been lost

#### Scenario: State written by another tool
- **WHEN** a `claude` from `[tools]` writes `.claude` in the project's `HOME` of a mode 2 sandbox and `up` runs again
- **THEN** the files are unchanged

#### Scenario: Agent verbs from the agent tree
- **WHEN** the agent runs `devcroft agent progress <message>`
- **THEN** the report is recorded for the sandbox

### Requirement: Commands reach the right tree and preserve OSC 7501 transparency
`exec`, `shell`, `ssh` and services SHALL reach the project tree. `agent acp`
and `agent run` SHALL reach the agent tree through its own control socket
(using innermost-root discovery, bounded `ACP_AUTO_UP_DEADLINE`, and valid
`devcroft up [--name <n>]` remedies). Interactive PTY sessions on the agent
tree (`agent run`) SHALL satisfy the same bidirectional OSC 7501 transparency
contract as mode 1 (the `exec` capability's, with zero OSC sequences emitted on
`agent acp`'s `stdout`). `status`, `logs`,
`policy --render` and `why` SHALL report both trees, labelled.

#### Scenario: Agent launcher in the project's shell
- **WHEN** the user types the agent's command in `devcroft shell` on a mode 2 sandbox
- **THEN** the command is not found and `devcroft shell` has said to use `devcroft agent run`

#### Scenario: Bidirectional OSC 7501 over the agent tree's PTY
- **WHEN** an agent started via `devcroft agent run` in mode 2 emits a capability query immediately upon spawn and the host terminal replies without a newline
- **THEN** the agent receives the reply in the agent tree without waiting for Enter and its state reports reach the terminal verbatim

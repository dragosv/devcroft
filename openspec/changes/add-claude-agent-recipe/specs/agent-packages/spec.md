## ADDED Requirements

### Requirement: Script-free Claude adapter installation

This change SHALL extend `agent-packages` materialization with the `node-tarball` and `npm-lock` v3 layouts without running downloaded code on the host. The Claude recipe SHALL pin each component and platform payload, verify every lockfile tarball, declare `state_paths = [".claude", ".claude.json"]` and Linux-scoped `procfs = true`, and refuse any selected install script unless its omission is qualified. Interactive and ACP launches SHALL use their respective pinned executables, reserve `CLAUDE_CODE_EXECUTABLE` on every launch path, and keep session headers and credentials out of child arguments.

#### Scenario: Both entrypoints use the same sandbox
- **WHEN** the CLI and adapter are launched successively in either order
- **THEN** both work from the verified read-only object, preserving settings, login and history across launches

#### Scenario: Inherited CLAUDE_CODE_EXECUTABLE cannot hijack the adapter
- **WHEN** the provider's environment exports `CLAUDE_CODE_EXECUTABLE` pointing to a marker-writing executable and `devcroft agent acp claude-code` runs
- **THEN** `CLAUDE_CODE_EXECUTABLE` is stripped before the adapter starts, the marker executable is not run, and no MCP headers appear in child `argv`

#### Scenario: Activation script exports a reserved variable
- **WHEN** the provider's activation script exports `CLAUDE_CODE_EXECUTABLE` and the agent is started by `agent acp`, by `agent run` and by its command in `devcroft shell`
- **THEN** the variable is absent from the agent's environment in all three

### Requirement: Subscription login and multi-mode authentication
Using the deterministic authentication-mode precedence from `agent-packages`, a recipe MAY qualify `login` (subscription/OAuth) alongside `api-key` (and `env` for a separately qualified non-brokerable recipe), optionally scoped per platform. Claude SHALL advertise only qualified `api-key` or `login` modes on each platform. Under `login` no broker and no key SHALL be required, login hosts SHALL join the egress allowlist, and `up` SHALL announce that the token in `HOME` is readable by the project. Under `env`, `up` SHALL print the delivered-credential custody disclosure. When the selected mode's prerequisite is missing (or unsupported on this platform), `up` SHALL fail at layer `config` naming it and the other modes the recipe supports on this platform.

#### Scenario: Brokered API key on an empty allowlist
- **WHEN** a project with `network.allow = []` is brought up with `[agent] name = "claude-code"` under `api-key` and `ANTHROPIC_API_KEY` exported
- **THEN** `up` succeeds and the agent's request reaches the API through the broker

#### Scenario: Subscription login without an API key
- **WHEN** `ANTHROPIC_API_KEY` is unset and the user runs `up` with `[agent] name = "claude-code"` and `auth = "login"`
- **THEN** `up` succeeds without a broker, allows the recipe's API and login hosts, and announces `HOME` token visibility

#### Scenario: Exported key does not change the mode
- **WHEN** a sandbox recorded with `auth = "login"` is brought up from a shell that exports `ANTHROPIC_API_KEY`
- **THEN** it stays in `login` mode and the sandbox does not receive the key

#### Scenario: Switching recipes selects authentication for the new recipe
- **WHEN** the global selection specifies Claude with login and an OpenCode sandbox using a brokered key is recreated with `--agent claude-code`
- **THEN** Claude uses login, and switching back to OpenCode selects its qualified broker mode

#### Scenario: Missing key names the alternatives
- **WHEN** the mode is `api-key`, the key variable is unset and no credential command is configured
- **THEN** `up` fails at layer `config` naming `ANTHROPIC_API_KEY`, `[agent.credentials.claude-code]` and `--agent-auth login`

#### Scenario: Default allow refuses broker
- **WHEN** the project's `network.default` is `"allow"` and the mode is `api-key`
- **THEN** `up` fails at layer `config` naming the agent and `--no-agent`

### Requirement: Login credentials and managed hooks

The recipe SHALL apply credential collision checks to every supported mode and merge the Claude denial hook under the sandbox HOME using the package change's safe update contract. The project SHALL NOT select a credential source.

#### Scenario: Forwarded key under login
- **WHEN** `devcroft.toml` lists `ANTHROPIC_API_KEY` in `[env] forward` and the mode is `login`
- **THEN** `up` fails at layer `config` naming `[env] forward` and the recipe, rather than letting the API key replace the subscription

#### Scenario: User broker for the same host under login
- **WHEN** the user configuration brokers `api.anthropic.com` and the mode is `login`
- **THEN** `up` fails at layer `config` naming the broker entry, since the brokered host would not be reachable directly

#### Scenario: Existing Claude preferences
- **WHEN** the user enables the recipe with an existing personal settings file
- **THEN** the managed hook is present once and existing preferences and login state survive restart

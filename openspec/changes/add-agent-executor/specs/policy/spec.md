## ADDED Requirements

### Requirement: Separate agent and project policies, isolated authentication and overlap rejection
A mode 2 sandbox SHALL require `network.default = "deny"` and SHALL compile two
profiles, both shown by `policy --render` with origins. The agent profile SHALL
hold the workspace, the agent's object and separate `HOME`, and the agent's
active hosts (including login hosts under `auth = "login"`). The project
profile SHALL hold today's rules and none of the agent's hosts or object
grants. Canonical resolved grants in every project profile (`AccessMode::Read`
as well as `AccessMode::ReadWrite`) SHALL be checked before startup against
`~/.local/share/devcroft-agent-homes` and the agent-sockets directory; any
grant containing or contained by either root SHALL fail `up` at layer `config`
naming the grant. Each tree SHALL have its own egress proxy, allowlist, token,
socket and log, and every authentication credential and route of the recipe
(`api-key` broker, `login` tokens in agent `HOME`, `env` delivered key, proxy
token and proxy socket) SHALL exist only in the agent tree.

#### Scenario: Project has no direct access to the agent's credentials or route
- **WHEN** a project build script searches its own environment, accessible files and logs for any agent token or placeholder, inspects the agent processes' environment and arguments for any real upstream credential (`api-key` value, `auth = "env"` value or `auth = "login"` token), and tries to connect to the agent's proxy port and proxy socket
- **THEN** it finds no real upstream credential anywhere, finds no agent token or placeholder in its own environment, files or logs, and both the agent proxy port and socket are unreachable

#### Scenario: Project read grant overlapping agent HOME is refused at config
- **WHEN** `devcroft.toml` declares `filesystem.read` covering `~/.local/share/devcroft-agent-homes`, or a symlink that resolves into it
- **THEN** `up` fails at layer `config` before starting either tree, naming the overlapping read grant

#### Scenario: Agent cannot use the project's allowlist directly
- **WHEN** the agent process connects to a host only the project's allowlist permits
- **THEN** the connection is refused for the agent process

#### Scenario: Both directions are stated
- **WHEN** `policy --render` shows a mode 2 sandbox
- **THEN** it states that commands sent through the executor run with the project's policy, and that project code can write workspace files the agent runs or obeys

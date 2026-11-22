## ADDED Requirements

### Requirement: Separate agent and project policies
A mode 2 sandbox SHALL compile two profiles, both shown by `policy --render`
with origins. The agent profile SHALL hold the workspace, the agent's object
and home, and the agent's hosts. The project profile SHALL hold today's
rules and none of the agent's hosts. Each tree SHALL have its own egress
proxy, allowlist, token, socket and log. The agent's credential route (broker
endpoint and placeholder, proxy token, proxy socket) SHALL exist only in the
agent tree.

#### Scenario: Project cannot use the agent's route
- **WHEN** a project build script looks for the agent's token and placeholder in its environment, files and logs, and tries the agent's proxy socket
- **THEN** it finds neither and the socket is not reachable

#### Scenario: Agent cannot use the project's allowlist directly
- **WHEN** the agent process connects to a host only the project's allowlist permits
- **THEN** the connection is refused for the agent process

#### Scenario: Authority union is stated
- **WHEN** `policy --render` shows a mode 2 sandbox
- **THEN** it states that commands sent through the executor run with the project's policy

## ADDED Requirements

### Requirement: Agent grants carry their own origin
Every rule an agent recipe adds SHALL carry origin `agent:<recipe>` and SHALL
appear in `policy --render`. These are: read-only access to its object and
the host libraries its runtime needs, write access to its state directory,
and its network destinations. No agent rule SHALL grant `/usr/bin` or write
access inside the artifact store.

#### Scenario: Rendered agent rules
- **WHEN** a sandbox with the `claude` recipe runs `policy --render`
- **THEN** the object, the state directory and `api.anthropic.com` appear with origin `agent:claude`

#### Scenario: No agent, no agent rules
- **WHEN** a sandbox is brought up without an agent
- **THEN** no rule with an `agent:` origin is compiled

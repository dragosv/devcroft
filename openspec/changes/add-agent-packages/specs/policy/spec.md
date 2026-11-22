## ADDED Requirements

### Requirement: Agent rules carry their own origin
Every rule an agent recipe adds SHALL carry origin `agent:<recipe>` and SHALL
appear in `policy --render`. These are: read-only access to its object, its
network and login destinations, and its environment variables. No agent rule
SHALL grant `/usr/bin`, write access inside the artifact store, or any path
under devcroft's data directory.

#### Scenario: Rendered agent rules
- **WHEN** a sandbox with the `claude-code` recipe runs `policy --render`
- **THEN** the object and `api.anthropic.com` appear with origin `agent:claude-code`

#### Scenario: No agent, no agent rules
- **WHEN** a sandbox is brought up without an agent
- **THEN** no rule with an `agent:` origin is compiled

### Requirement: Every up check reads the composed policy
`up` SHALL compose the agent's rules with the manifest's policy and the
provider's grants before any check that reads the policy, including the
broker checks, the egress-proxy decision, rendering and compilation. Selecting
an agent SHALL be the only user-configuration input that adds egress hosts,
limited to the recipe's own, and `up` SHALL print them on every run.

#### Scenario: Agent host on an empty allowlist
- **WHEN** a project with `network.allow = []` is brought up with an agent
- **THEN** the egress proxy starts and allows the recipe's hosts only

#### Scenario: Egress widening announced
- **WHEN** `up` runs with an agent selected
- **THEN** it prints the hosts the recipe added to the sandbox's egress

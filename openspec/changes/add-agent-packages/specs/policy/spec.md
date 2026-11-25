## ADDED Requirements

### Requirement: Agent rules carry their own origin
Every rule an agent recipe adds SHALL carry origin `agent:<recipe>` and SHALL
appear in `policy --render`. These are: read-only access to its object,
read-only access to `<project>/.devcroft/<name>/agent/bin` (so the launcher is
mounted and executable even when the manifest does not grant `.`), its API
network destinations, and its `[launch.env]` variables, shown as reaching the agent only. When
`auth = "api-key"` is selected, the brokered host SHALL also carry
`credential:broker`. No agent rule SHALL grant `/usr/bin`, write access inside
the artifact store, or any path under devcroft's data directory.

#### Scenario: Rendered agent rules and launcher grant without project root grant
- **WHEN** a sandbox whose `devcroft.toml` does not grant `.` brings up the `opencode` recipe, runs `policy --render`, and invokes `opencode` on `PATH`
- **THEN** the object, `.devcroft/<name>/agent/bin` and the qualified provider host appear with origin `agent:opencode`, and the launcher executes

#### Scenario: No agent, no agent rules
- **WHEN** a sandbox is brought up without an agent
- **THEN** no rule with an `agent:` origin is compiled

### Requirement: Every up check reads the composed policy
`up` SHALL compose the agent's rules with the manifest's policy, the provider's
grants and the tooling layer before any check that reads the policy, including
the broker checks, the egress-proxy decision, rendering and compilation.
Selecting an agent SHALL be the only user-configuration input that adds egress
hosts, limited to the recipe's own active hosts, and `up` SHALL print them on
every run.

#### Scenario: Agent host on an empty allowlist
- **WHEN** a project with `network.allow = []` is brought up with an agent
- **THEN** the egress proxy starts and allows only the recipe's active hosts

#### Scenario: Egress widening announced
- **WHEN** `up` runs with an agent selected, including on a sandbox already up
- **THEN** it prints the hosts the recipe added to the sandbox's egress

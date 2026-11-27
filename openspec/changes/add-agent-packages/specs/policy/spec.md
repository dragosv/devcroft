## ADDED Requirements

### Requirement: Agent rules carry their own origin
Every rule an agent recipe adds SHALL carry origin `agent:<recipe>` and SHALL
appear in `policy --render`. These are: read-only access to its object, any
library or loader path the recipe declares for the current platform, its API
network destinations, and its `[launch.env]` variables, shown as reaching the
agent only. When `auth = "api-key"` is selected, the brokered host SHALL also
carry `credential:broker`. No agent rule SHALL grant `/usr/bin`, write access
inside the artifact store, any path under devcroft's data directory, or a
`PATH` entry.

#### Scenario: Rendered agent rules without a project root grant
- **WHEN** a sandbox whose `devcroft.toml` does not grant `.` brings up the `opencode` recipe, runs `policy --render`, and starts the agent with `devcroft agent run`
- **THEN** the object and the qualified provider host appear with origin `agent:opencode`, and the agent starts

#### Scenario: No agent, no agent rules
- **WHEN** a sandbox is brought up without an agent
- **THEN** no rule with an `agent:` origin is compiled

### Requirement: Each up check reads the policy it is specified to read
Before teardown and provider resolution, `up` SHALL check the network
prerequisites (the broker checks, the egress-proxy decision and the recipe's
hosts) against the manifest's network policy composed with the recipe's active
hosts, and the agent's executable against the agent's own grants and the
baseline. Provider grants and the tooling layer add only filesystem paths,
never hosts, so these checks need no provider resolution. `policy --render`,
`why` and compilation SHALL all use one final policy that composes the
manifest, the agent's rules, the provider's grants and the tooling layer.
Selecting an agent SHALL be the only user-configuration input that adds egress
hosts, limited to the recipe's own active hosts, and `up` SHALL print them on
every run.

#### Scenario: Agent host on an empty allowlist
- **WHEN** a project with `network.allow = []` is brought up with an agent
- **THEN** the egress proxy starts and allows only the recipe's active hosts

#### Scenario: Broker refused before the provider runs
- **WHEN** the recipe's key is missing on a project whose provider would take minutes to resolve
- **THEN** `up` fails at layer `config` without resolving the provider

#### Scenario: Render matches enforcement
- **WHEN** a sandbox with an agent and a closure provider is up
- **THEN** `policy --render` lists both the `agent:` and the `provider:` rules, and `why` on the agent's object and on a store path agrees with the kernel

#### Scenario: Egress widening announced
- **WHEN** `up` runs with an agent selected, including on a sandbox already up
- **THEN** it prints the hosts the recipe added to the sandbox's egress

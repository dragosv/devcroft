## ADDED Requirements

### Requirement: Checks run before teardown
Before `up` terminates a keeper, resolves the provider or creates state, it
SHALL run every check that does not need the resolved environment: the agent
selection, its recipe and platform, authentication and collisions, the user's
`[credentials]` and broker entries, and host-side requirements; and it SHALL
materialize a missing agent object. A sandbox that is already up and is not
being recreated SHALL NOT be failed by these checks.

#### Scenario: Missing key on recreate
- **WHEN** a healthy sandbox is recreated from a shell in which a broker's key is unset and no credential command is configured
- **THEN** `up` fails at layer `config` and the sandbox is still up

#### Scenario: Download fails on recreate
- **WHEN** a healthy sandbox is recreated with a recipe whose artifact cannot be downloaded
- **THEN** `up` fails at layer `provider` and the sandbox is still up

#### Scenario: No provisioning for a failed check
- **WHEN** a check fails on a sandbox that is down
- **THEN** the provider was not resolved and no state was created

#### Scenario: Already up without the key
- **WHEN** plain `up` runs on a healthy sandbox from a shell without the key
- **THEN** it reports that the sandbox is already up

### Requirement: A selection change on an existing sandbox requires recreate
When `Meta` already exists for a sandbox (whether running or stopped) and `--recreate` was not passed: if the effective selection equals `Meta`'s recorded state, a running sandbox SHALL report already up (printing any recorded agent and its hosts) and a stopped sandbox SHALL start with that recorded state; if a flag asks for a different selection (including adding an agent to an `Unset` or `Disabled` sandbox), `up` SHALL fail at layer `config` naming the same command with `--recreate`, and SHALL record nothing.

#### Scenario: Different agent on a running sandbox
- **WHEN** a sandbox runs `claude-code` and `up --agent opencode` is run
- **THEN** it fails naming `up --agent opencode --recreate`, and the running sandbox and its recorded selection are unchanged

#### Scenario: Opting out on a running sandbox
- **WHEN** a sandbox runs an agent and `up --no-agent` is run
- **THEN** it fails naming `up --no-agent --recreate`

#### Scenario: Auto-up keeps the selection
- **WHEN** `devcroft shell` starts a sandbox that was brought up with `--no-agent` and is down
- **THEN** it starts without an agent

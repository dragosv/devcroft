## ADDED Requirements

### Requirement: Prepare a replacement agent before teardown

For a start or recreation, devcroft SHALL validate the selection, platform, launch path, effective credentials and host-side requirements and acquire the verified object before replacing a keeper or resolving the provider. An unchanged healthy sandbox uses its recorded selection.

#### Scenario: Download fails on recreate
- **WHEN** a healthy sandbox is recreated with a recipe whose artifact cannot be downloaded
- **THEN** `up` fails at layer `provider` and the sandbox is still up

### Requirement: A selection change on an existing sandbox requires recreate
When `Meta` already exists for a sandbox (whether running or stopped) and `--recreate` was not passed: if the effective selection equals `Meta`'s recorded state, a running sandbox SHALL report already up (printing any recorded agent and its hosts) and a stopped sandbox SHALL start with that recorded state; if a flag asks for a different selection (including adding an agent to an `Unset` or `Disabled` sandbox), `up` SHALL fail at layer `config` naming the same command with `--recreate`, and SHALL record nothing.

#### Scenario: Different agent on a running sandbox
- **WHEN** a sandbox runs another recipe and `up --agent opencode` is run
- **THEN** it fails naming `up --agent opencode --recreate`, and the running sandbox and its recorded selection are unchanged

#### Scenario: Opting out on a running sandbox
- **WHEN** a sandbox runs an agent and `up --no-agent` is run
- **THEN** it fails naming `up --no-agent --recreate`

#### Scenario: Auto-up keeps the selection
- **WHEN** `devcroft shell` starts a sandbox that was brought up with `--no-agent` and is down
- **THEN** it starts without an agent

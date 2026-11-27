## ADDED Requirements

### Requirement: Validate before stopping a working sandbox

When starting or recreating a sandbox, explicit and automatic startup SHALL load the user configuration and check available credential and broker prerequisites, including running a configured credential command, before keeper or proxy termination, provider resolution and new sandbox state, preserving the failure's error layer and exit code (`config` exit code `2` for user-configuration, credential or broker errors). Without `--recreate`, an already healthy sandbox SHALL return already up (or proceed on automatic startup) without re-requiring startup credentials, even if the user configuration file was edited after startup.

#### Scenario: Missing key on recreate
- **WHEN** a healthy sandbox is recreated from a shell in which a broker's key is unset
- **THEN** `up` fails at layer `config` (exit code 2) and the sandbox is still up

#### Scenario: No provisioning for a failed check
- **WHEN** a check fails on a sandbox that is down
- **THEN** the provider was not resolved and no state was created

#### Scenario: Automatic startup preserves config error layer
- **WHEN** `devcroft shell` or `devcroft exec` auto-starts a down sandbox whose user configuration is invalid or lacks a required credential
- **THEN** it fails at layer `config` with exit code 2 rather than exit code 3, without resolving the provider or creating state

#### Scenario: Already up without the key or after editing user config
- **WHEN** plain `up` or `devcroft shell` runs on a healthy sandbox from a shell without the startup key, or after `~/.config/devcroft/config.toml` was edited
- **THEN** plain `up` reports that the sandbox is already up and `devcroft shell` attaches without restarting the keeper

### Requirement: A failure after teardown says what was lost
When a recreation fails after the old keeper or proxy was stopped, for example
during provider resolution, `up` SHALL say that the sandbox is now down and
SHALL keep the previously recorded state in `Meta`. It SHALL name the command
that starts that recorded state again, and SHALL NOT claim a rollback it did not
perform.

#### Scenario: Provider fails during recreation
- **WHEN** a healthy sandbox is recreated and provider resolution then fails
- **THEN** `up` fails at layer `provider`, says the sandbox is down, and names `devcroft up` to start the recorded state again

### Requirement: rm runs in the lifecycle critical section
`rm` SHALL read the sandbox's `Meta` and remove its state while holding that
sandbox's lifecycle lock, and SHALL sweep shared artifact objects (store
providers, agent objects, the pinned runtime) under the store's lease. Today
it reads `Meta` before taking the lock and sweeps after releasing it.

#### Scenario: rm racing up
- **WHEN** `rm` and `up` run concurrently on the same sandbox
- **THEN** either `rm` removes the state `up` recorded, or `up` creates it afresh after `rm`, and no object a surviving `Meta` references is swept


## ADDED Requirements

### Requirement: Validate before stopping a working sandbox

When starting or recreating a sandbox, explicit and automatic startup SHALL load the user configuration and check available credential and broker prerequisites before keeper or proxy termination, provider resolution and new sandbox state, preserving the failure's error layer and exit code (`config` exit code `2` for user-configuration, credential or broker errors). Without `--recreate`, an already healthy sandbox SHALL return already up (or proceed on automatic startup) without re-requiring startup credentials, even if the user configuration file was edited after startup.

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

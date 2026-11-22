## ADDED Requirements

### Requirement: The identity table
`devcroft.toml` SHALL accept an optional `[identity]` table with two keys:
`user`, a POSIX user name (default `devcroft`), and `mode`, one of `separate`
(default) or `shared`. Any other key, a name that is not a valid user name, the
name `root`, or another value of `mode` SHALL fail validation at layer
`config`, exit code 2, naming the key. The table SHALL NOT name a host uid,
gid or account: which host identities a sandbox may use is decided by the host
(subordinate ranges, helper-created accounts), never by the repository.

#### Scenario: Defaults
- **WHEN** a manifest has no `[identity]` table
- **THEN** the sandbox uses user `devcroft` in mode `separate`

#### Scenario: Root is refused
- **WHEN** `[identity]` sets `user = "root"`
- **THEN** validation fails at layer `config` with exit code 2 naming `user`

#### Scenario: A host uid cannot be requested
- **WHEN** `[identity]` contains `uid = 501`
- **THEN** validation fails at layer `config` with exit code 2 naming `uid`

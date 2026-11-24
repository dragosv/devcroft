## ADDED Requirements

### Requirement: The identity table
`devcroft.toml` SHALL accept an optional `[identity]` table with two keys:
`user`, a POSIX user name (default `devcroft`), and `mode`, which accepts only
`shared`, the default: `separate` is the user's choice, since it can leave
files in the project that the user cannot edit. Any other key, a name that is not a valid user name, the
names `root` and `nobody`, or another value of `mode` SHALL fail validation at layer
`config`, exit code 2, naming the key. The table SHALL NOT name a host uid,
gid or account: which host identities a sandbox may use is decided by the host
(subordinate ranges, helper-created accounts), never by the repository.

#### Scenario: Defaults
- **WHEN** a manifest has no `[identity]` table
- **THEN** the sandbox uses user `devcroft` in mode `shared`

#### Scenario: Root is refused
- **WHEN** `[identity]` sets `user = "root"`
- **THEN** validation fails at layer `config` with exit code 2 naming `user`

#### Scenario: A host uid cannot be requested
- **WHEN** `[identity]` contains `uid = 501`
- **THEN** validation fails at layer `config` with exit code 2 naming `uid`

#### Scenario: A manifest cannot select separate
- **WHEN** `[identity]` sets `mode = "separate"`
- **THEN** validation fails at layer `config` with exit code 2 naming `mode`

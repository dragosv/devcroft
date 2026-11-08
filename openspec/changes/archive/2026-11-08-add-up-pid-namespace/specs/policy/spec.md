# policy

## ADDED Requirements

### Requirement: /proc is granted only on a procfs the sandbox owns

The compiled policy SHALL grant `/proc` read-only, with origin
`pid-namespace`, exactly when the sandbox has its own PID namespace and fresh
procfs, and SHALL NOT grant it otherwise.

#### Scenario: A sandbox with its own PID namespace

- **WHEN** a sandbox runs in its own PID namespace
- **THEN** `policy --render` shows `/proc` with origin `pid-namespace`
- **AND** a session can read its own `/proc/self/status`

#### Scenario: A sandbox in the host's PID namespace

- **WHEN** a sandbox shares the host's PID namespace
- **THEN** `/proc` is not granted

# ssh Specification

## Purpose

Expose each sandbox as an SSH host so that unmodified SSH clients and
editors (VS Code Remote-SSH, Cursor, Zed, JetBrains Gateway, rsync, scp)
can work inside the sandbox. The SSH layer provides protocol compatibility;
the real access boundary is the state dir's filesystem permissions.

## ADDED Requirements

### Requirement: Embedded server inside the boundary
The system SHALL run an SSH server inside the keeper (inside the sandbox
boundary), listening ONLY on a unix socket in the sandbox state dir, mode
0600, within a 0700 state dir. The server MUST NOT bind any TCP port.

#### Scenario: No TCP exposure
- **WHEN** the sandbox is up
- **THEN** no listening TCP socket belongs to the keeper

### Requirement: ProxyCommand bridging
The system SHALL provide `devcroft proxy <host>` which parses
`<name>.devcroft` from `<host>`, connects to that sandbox's SSH socket, and
bridges it to stdio, exiting non-zero with a clear error when the sandbox
does not exist or is not up (unless auto-up applies).

#### Scenario: Editor connects by hostname
- **WHEN** `~/.ssh/config` contains the devcroft block
- **AND** the user opens `myproj.devcroft` in an SSH-capable editor
- **THEN** the editor reaches a shell/session inside the sandbox with no
  further configuration

### Requirement: ssh-config emission
The system SHALL provide `devcroft ssh-config` printing a single wildcard
`Host *.devcroft` block, and `--write` SHALL insert or update exactly one
marker-delimited managed section in `~/.ssh/config`, idempotently, never
touching content outside the markers.

#### Scenario: Idempotent write
- **WHEN** `ssh-config --write` runs twice
- **THEN** `~/.ssh/config` is identical after the second run

### Requirement: Key management
The system SHALL generate a client ed25519 keypair on first use, stored in
the data dir with mode 0600, and per-sandbox ephemeral host keys stored in
each sandbox's state dir. The client key SHALL be denied to all sandboxes by
baseline policy.

#### Scenario: First run generates keys
- **WHEN** no keypair exists and any ssh-related command runs
- **THEN** the keypair is created before proceeding, with a one-line notice

### Requirement: SSH feature coverage for editors
The system SHALL support, at minimum: exec channel, pty/shell channel,
window-change, env passthrough of an allowlist (TERM, LANG, LC_*), exit
status, and direct-tcpip local forwarding (`-L`) restricted to targets the
policy allows. SFTP SHALL be supported sufficiently for scp/rsync and editor
file operations.

#### Scenario: Remote-SSH server bootstrap
- **WHEN** VS Code Remote-SSH connects to `<name>.devcroft`
- **THEN** its server bootstrap (exec + sftp + port forward) completes and
  the workspace opens at the project root

#### Scenario: Agent forwarding is off by default
- **WHEN** the user connects without `ssh.forward_agent = true`
- **THEN** no agent socket exists inside the sandbox

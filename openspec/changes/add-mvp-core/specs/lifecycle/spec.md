# lifecycle Specification

## Purpose

Manage the resident keeper process per sandbox: creation, idempotent
startup, health, teardown, and crash recovery. The keeper is the spawn
server through which all sessions enter the sandbox.

## ADDED Requirements

### Requirement: Listener-before-restriction ordering
The system SHALL create all listener sockets before applying sandbox
restrictions, and SHALL apply restrictions to the keeper such that the
keeper and all its descendants are inside the boundary while the sockets
remain reachable from outside.

Every listener socket the system creates SHALL be mode 0600, inside the
0700 state dir. The `ssh` spec states this for the SSH socket; it applies
to the **control** socket equally, and if anything more so — the control
socket carries the spawn protocol, so it is the more sensitive of the
two, not the less.

Stated explicitly because its absence was a real gap: the control socket
was left at whatever the umask produced (0755 in practice) while the SSH
socket set 0600 explicitly, so the 0700 state dir was the only thing
protecting the more sensitive of the two. That directory mode is applied
only to a state root the system itself creates, so a root predating it
keeps its old permissions — leaving nothing behind the socket at all.

#### Scenario: Keeper cannot widen its own boundary
- **WHEN** the keeper is running
- **AND** code inside the sandbox attempts to spawn a process outside the
  restriction set
- **THEN** the kernel denies it; there is no API path to escape

#### Scenario: Both sockets are 0600
- **WHEN** a sandbox is up, at either isolation tier
- **THEN** the control socket and the SSH socket are both mode 0600, and
  the state dir containing them is 0700

### Requirement: Idempotent up
The system SHALL make `devcroft up` idempotent: if a healthy keeper exists,
report it and exit 0; if a dead keeper left state behind (stale pid file,
orphan sockets), clean up and start fresh; `--recreate` SHALL force a full
teardown, re-resolution of the environment, and recompilation of policy.

#### Scenario: Up on a healthy sandbox
- **WHEN** the keeper is alive and responsive
- **THEN** `up` prints the existing status and exits 0 without side effects

#### Scenario: Recovery after host reboot
- **WHEN** state exists but the pid is dead and sockets are orphaned
- **THEN** `up` removes stale state, starts a new keeper, and notes the
  recovery in one line

### Requirement: Suspend/resume survival
The system SHALL ensure the keeper survives host suspend/resume, and that
the first command after resume transparently verifies keeper health before
proceeding.

#### Scenario: First command after resume
- **WHEN** the host resumes and the keeper is still alive
- **THEN** `shell` connects with no user-visible difference

### Requirement: Teardown
The system SHALL provide `down` (stop keeper, keep state and compiled
policy) and `rm` (stop keeper, remove all state for the sandbox). Both
SHALL terminate the entire session process tree, escalating SIGTERM to
SIGKILL after a grace period.

#### Scenario: Down with live sessions
- **WHEN** two interactive sessions are open and the user runs `down`
- **THEN** sessions receive SIGTERM, then SIGKILL after the grace period,
  and the keeper exits last

### Requirement: Status and logs
The system SHALL provide `status` (keeper health, uptime, session count,
environment staleness, degraded capabilities) and `logs` (keeper log tail,
including session spawn/exit records with timestamps).

#### Scenario: Status of a stale environment
- **WHEN** the flox manifest changed after last `up`
- **THEN** `status` shows `env: stale` alongside `keeper: healthy`

### Requirement: Hooks run inside the boundary
The system SHALL execute `hooks.post_create` inside the sandbox as the
first session after the first successful `up` (and after `--recreate`),
and `hooks.post_start` inside the sandbox on every keeper start, both
subject to the full filesystem and network policy. Hooks are project code
and are never granted provisioning privileges. A failing hook SHALL fail
`up` with layer `keeper` unless `--skip-hooks` is given, and hook output
SHALL appear in `logs`.

#### Scenario: Hook output survives the keeper writing the same log
- **WHEN** a hook produces output while the keeper writes its own
  spawn/exit records to the same log file
- **THEN** `logs` contains the hook's output as lines of its own, with
  neither writer's records overwritten or split by the other's

#### Scenario: Hook needs an allowlisted domain
- **WHEN** `post_create = "cargo fetch"` and `network.allow` includes
  `crates.io`
- **THEN** the hook succeeds through the policy like any session would

#### Scenario: Hook blocked by deny-all
- **WHEN** `post_create` requires network and the allowlist is empty
- **THEN** the hook fails, `up` fails with layer `keeper`, and the error
  names the hook — the policy is not widened for hooks

### Requirement: Concurrent sandboxes
The system SHALL support multiple sandboxes on one host with fully disjoint
state dirs and sockets, and SHALL document (not hide) that MVP provides no
process-visibility separation between sandboxes.

#### Scenario: Two sandboxes side by side
- **WHEN** sandboxes `a` and `b` are both up
- **THEN** `exec` against each reaches its own environment and policy, and
  `ps` lists both keepers with distinguishable names

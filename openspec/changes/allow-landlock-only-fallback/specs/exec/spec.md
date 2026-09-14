## MODIFIED Requirements

### Requirement: Auto-up convenience
The system SHALL, when `exec` or `shell` targets a sandbox that is not up,
start it first (equivalent to `up`) unless `--no-up` is given. Auto-up
SHALL NOT accept `--allow-degraded-isolation`: on a host where `up` would
need it, auto-up fails with `up`'s own message, naming the flag and the
command to run it with.

#### Scenario: Shell on a cold sandbox
- **WHEN** no keeper is running and the user runs `shell`
- **THEN** the sandbox comes up, then the shell attaches, with the `up`
  output preceding the prompt

#### Scenario: Shell on a cold sandbox, on a host that needs the opt-in
- **WHEN** no keeper is running, the host cannot create an unprivileged
  mount namespace, and the user runs `shell` or `exec`
- **THEN** the command fails at layer `backend` without starting
  anything
- **AND** the message tells the user to run `up --allow-degraded-isolation`
  explicitly, or to fix the host

# exec Specification

## Purpose

Run commands and interactive shells inside a running sandbox through the
keeper's spawn protocol, with correct terminal, signal, and exit-code
semantics.

## ADDED Requirements

### Requirement: One-shot execution
The system SHALL provide `devcroft exec [name] -- <cmd> [args...]` which
spawns the command inside the sandbox with the resolved environment, streams
stdio, and returns the command's exit code as its own.

#### Scenario: Exit code propagation
- **WHEN** the user runs `exec -- sh -c 'exit 42'`
- **THEN** devcroft exits with code 42

#### Scenario: Working directory mapping
- **WHEN** the user runs `exec` from `<root>/src/`
- **THEN** the command starts in `<root>/src/` inside the sandbox

### Requirement: Interactive shell
The system SHALL provide `devcroft shell [name]` allocating a pty, running
the user's shell (respecting `$SHELL` if it is inside the allowed policy,
else falling back to `/bin/sh`), with window-resize and job-control support.
The pty is allocated when the client's stdin is a terminal; otherwise the
shell runs on pipes, as `ssh` decides, so input piped to it ends the session
at its end.

#### Scenario: Input that is not a terminal
- **WHEN** `devcroft shell` runs with stdin redirected from a file, a pipe or
  `/dev/null`
- **THEN** the shell reads that input and the session ends when it does
- **AND** no pty is allocated, since there is no terminal to give it

#### Scenario: Resize propagation
- **WHEN** the terminal window is resized during a shell session
- **THEN** the pty inside the sandbox receives the new dimensions

### Requirement: Signal forwarding
The system SHALL forward SIGINT, SIGTERM, and SIGHUP from the client to the
session process group, and SHALL reap sessions whose client disconnected
uncleanly.

#### Scenario: Ctrl-C reaches the child
- **WHEN** the user presses Ctrl-C during `exec -- sleep 100`
- **THEN** the sleep receives SIGINT and devcroft exits 130

#### Scenario: The session ignores a forwarded termination
- **WHEN** the client forwards SIGTERM or SIGHUP and the session does not
  exit within the grace period (an interactive shell ignores SIGTERM)
- **THEN** the client closes the connection and exits `128 + signal`
- **AND** the keeper reaps the session as for any client that went away

#### Scenario: Client killed mid-session
- **WHEN** the client process is killed with SIGKILL
- **THEN** the keeper terminates the orphaned session within the grace
  period and logs the event

### Requirement: Auto-up convenience
The system SHALL, when `exec` or `shell` targets a sandbox that is not up,
start it first (equivalent to `up`) unless `--no-up` is given.

#### Scenario: Shell on a cold sandbox
- **WHEN** no keeper is running and the user runs `shell`
- **THEN** the sandbox comes up, then the shell attaches, with the `up`
  output preceding the prompt

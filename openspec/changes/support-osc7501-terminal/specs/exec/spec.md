## ADDED Requirements

### Requirement: Bidirectional OSC 7501 transparency
Every interactive PTY session of the exec client (`devcroft shell`, and any
later client such as `devcroft agent run`) SHALL carry OSC 7501 sequences
byte-for-byte in both directions between the sandboxed process and the attached
terminal:
- Fragments split across reads and keeper frames, and several sequences in one
  buffer, SHALL arrive intact and in order, without waiting for a newline, the
  end of a sequence or process exit, and without filtering unknown fields.
- devcroft SHALL NOT answer a capability query itself, SHALL NOT emit agent
  identity or state for any process, SHALL NOT grant anything from a payload,
  and SHALL deliver terminal input only to the session of that connection.
- Log viewing (`devcroft logs` when `stdout` is a TTY) and replay SHALL strip
  ESC/OSC control sequences so recorded hook or provisioning output is never
  emitted as live OSC 7501 traffic, and live PTY session streams SHALL NOT be
  written to `paths.log`.

#### Scenario: Query right after spawn, reply without a newline
- **WHEN** a process sets its TTY raw, emits a capability query as its first output, and the terminal replies without a newline
- **THEN** the terminal receives the query intact and the process receives the reply without Enter and without echo

#### Scenario: Fragmented and coalesced sequences
- **WHEN** a query, its reply and several reports interleaved with ordinary output are split at arbitrary byte boundaries
- **THEN** both sides receive the exact byte stream in order

#### Scenario: Unknown values
- **WHEN** a report carries fields or values that devcroft does not know
- **THEN** its bytes are unchanged

#### Scenario: Silent terminal and non-OSC programs
- **WHEN** the terminal never replies, or a program emits no OSC 7501 and exits 0
- **THEN** devcroft produces no reply, identity or state

#### Scenario: Concurrent and successive sessions
- **WHEN** two terminals are attached to one sandbox, or one session ends and another starts
- **THEN** each process receives only the bytes of its own connection

#### Scenario: Viewing logs on a terminal strips recorded OSC sequences
- **WHEN** a sandbox hook wrote an OSC 7501 sequence to `paths.log` and the user runs `devcroft logs` in a terminal
- **THEN** the raw OSC escape sequence is stripped/escaped before reaching the terminal

### Requirement: Local terminal mode is safe and checked
The exec client SHALL keep normal signal handling while waiting for the keeper's `SpawnOk`, and after `SpawnOk` SHALL block forwarded signals (`SIGINT`, `SIGTERM`, `SIGHUP`, plus `SIGWINCH` on PTYs) before enabling raw mode and before reading or writing any session byte. It SHALL retry `tcsetattr` on `EINTR`, fail the session if a TTY cannot enter raw mode, and restore the terminal's original settings on every observable exit, including forwarded termination signals.

#### Scenario: Interrupting a client that waits on the keeper
- **WHEN** the keeper does not answer the spawn and the user presses Ctrl-C
- **THEN** the client ends and the terminal's attributes are the original ones

#### Scenario: Spawn refused
- **WHEN** the keeper refuses the spawn
- **THEN** the error is printed in the terminal's original mode

#### Scenario: Termination during a session
- **WHEN** the client receives SIGTERM or SIGHUP while in raw mode
- **THEN** the original attributes are restored before it exits

#### Scenario: Raw mode fails
- **WHEN** setting raw mode on a TTY fails for a reason other than `EINTR`
- **THEN** the session ends with an error instead of running in cooked mode

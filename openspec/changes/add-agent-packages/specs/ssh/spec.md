## ADDED Requirements

### Requirement: OSC 7501 transparency over SSH PTY sessions
A `devcroft ssh` session with a PTY SHALL carry OSC 7501 sequences in both
directions under the `exec` capability's OSC 7501 requirement, across channel
packets of any size. The local terminal's mode is the SSH client's.

#### Scenario: Negotiation over SSH
- **WHEN** a process in an SSH PTY session emits a capability query and the terminal replies without a newline
- **THEN** the query, the reply and later reports arrive unmodified and in order

#### Scenario: Packets split a sequence
- **WHEN** a sequence spans several channel packets
- **THEN** it is delivered intact without waiting for its end

#### Scenario: Two SSH sessions
- **WHEN** two SSH sessions with PTYs are open on one sandbox
- **THEN** each receives only its own terminal's bytes

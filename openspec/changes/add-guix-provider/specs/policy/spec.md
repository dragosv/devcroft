## ADDED Requirements

### Requirement: guix store grant without daemon authority
A guix sandbox SHALL receive `/gnu/store` read-only with origin
`provider:guix`, shown by `policy --render`, and SHALL NOT be able to reach
the guix daemon socket. On Linux the socket SHALL be absent from the
sandbox's view.

#### Scenario: Daemon socket unreachable
- **WHEN** a session in a guix sandbox connects to the guix daemon socket path
- **THEN** the connection fails because the path does not exist in the view

#### Scenario: Closure build needs no host libraries
- **WHEN** the sample builds from its guix closure
- **THEN** the compiled profile holds no grant under `/lib`, `/usr/lib` or `/usr/bin`
- **AND** the host `gcc` does not resolve inside

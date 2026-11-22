## ADDED Requirements

### Requirement: Attributed proto grants
Read-only grants for the proto object's tool directories and for the host
runtime those tools link against SHALL carry origin `provider:proto` and SHALL
appear in `policy --render`. No grant SHALL name `/usr/bin`, and no write grant
SHALL fall inside the proto object.

#### Scenario: Runtime write to the object refused
- **WHEN** a session tries to write inside the proto object
- **THEN** the write is denied and `why` names the read-only `provider:proto` rule

#### Scenario: Host compiler not reachable
- **WHEN** a session in a proto sandbox runs `cc`
- **THEN** the host compiler does not resolve

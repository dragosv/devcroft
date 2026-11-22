## ADDED Requirements

### Requirement: Attributed hermit grants
Read-only grants for the hermit object's package directories and for the host
runtime those packages link against SHALL carry origin `provider:hermit` and
SHALL appear in `policy --render`. No grant SHALL name `/usr/bin`, and a
manifest write grant overlapping the artifact store SHALL be refused at layer
`config`.

#### Scenario: Runtime write to the object refused
- **WHEN** a session tries to write inside the hermit object
- **THEN** the write is denied and `why` names the read-only `provider:hermit` rule

# lifecycle Delta Specification (add-macos-service-vm)

## ADDED Requirements

### Requirement: Guest lifetime is devcroft's, and lazy
The system SHALL start the shared guest only when a sandbox that needs it
comes up, and SHALL NOT require it for a sandbox that declares no services.

#### Scenario: No services, no guest
- **WHEN** a sandbox declaring no services comes up
- **THEN** no guest is started

### Requirement: Teardown keeps data, removal deletes it
`down` SHALL stop a sandbox's services and its host-side endpoints while
preserving its data volume. `rm` SHALL delete the data volume.

#### Scenario: down preserves data
- **WHEN** `down` runs for a sandbox with services in the guest
- **THEN** its services stop and its data volume remains

#### Scenario: rm deletes data
- **WHEN** `rm` runs for that sandbox
- **THEN** its data volume is deleted

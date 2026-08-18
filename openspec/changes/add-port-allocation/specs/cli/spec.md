# cli Delta Specification (add-port-allocation)

## ADDED Requirements

### Requirement: status reports allocated ports
The system SHALL report each allocated port and the variable carrying it
through `status`, so a user or agent can connect to a value it never
chose. Reporting the sandbox as healthy without reporting where its
services can be reached SHALL be treated as incomplete.

#### Scenario: Port and variable both shown
- **WHEN** `status` runs for a sandbox with an allocated port
- **THEN** it shows the variable name and the port number together

#### Scenario: No allocation, no noise
- **WHEN** `status` runs for a sandbox with no allocation requested
- **THEN** no allocation line is printed

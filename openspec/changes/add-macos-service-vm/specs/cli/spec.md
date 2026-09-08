# cli Delta Specification (add-macos-service-vm)

## ADDED Requirements

### Requirement: The service runtime is visible
The system SHALL state, once at `up` and in `status`, when a sandbox's
services run in a shared guest rather than on the host, and SHALL name the
host-side endpoint for reaching them.

The system SHALL NOT imply that host-side code gained a stronger boundary
because a guest is in use.

#### Scenario: up names where services run
- **WHEN** a sandbox with declared services comes up on macOS with the
  guest runtime
- **THEN** the output states that services run in the shared guest

#### Scenario: No implied containment
- **WHEN** the guest runtime is in use
- **THEN** no output describes the sandbox's own processes as running in
  the guest or as more isolated than Seatbelt provides

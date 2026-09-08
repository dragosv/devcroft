# config Delta Specification (add-macos-service-vm)

## ADDED Requirements

### Requirement: Linux service environment resolution is recorded separately
The system SHALL resolve a sandbox's service environment for the guest's
platform separately from the host toolchain environment, and SHALL record
the two as distinct resolutions with distinct fingerprints.

The system SHALL NOT represent both as one resolution, because a single
recorded environment that means different things on the host and in the
guest cannot answer a staleness question correctly.

#### Scenario: Two fingerprints
- **WHEN** a sandbox resolves a host toolchain and guest services
- **THEN** each resolution is recorded with its own fingerprint

#### Scenario: Guest staleness is answerable
- **WHEN** the service definition changes but the toolchain definition does
  not
- **THEN** `status` reports the environment stale

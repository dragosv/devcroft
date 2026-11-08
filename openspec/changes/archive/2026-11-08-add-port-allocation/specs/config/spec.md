# config Delta Specification (add-port-allocation)

## ADDED Requirements

### Requirement: Port allocation request
The system SHALL accept a port-allocation request in `[network]`,
validated with the same strictness as neighbouring keys: unknown keys
rejected with the full key path, malformed values failing at layer
`config` with exit code 2. Omitting the request SHALL leave behaviour
identical to a manifest written before this capability existed.

A request SHALL identify **both the service whose generated
configuration receives the port and the environment variable within it
that carries the value**, except for a request explicitly scoped to
sessions rather than to a service. Naming a variable alone SHALL NOT be
accepted: devcroft substitutes the value into the `environment` block it
generates for one declared service, so a request that does not say which
service cannot be substituted, cannot be validated against that
service's command, and cannot name the service in the failure the
`port-allocation` spec requires.

#### Scenario: Malformed request rejected
- **WHEN** the allocation request is not of the accepted shape
- **THEN** validation fails at layer `config` with exit code 2, naming
  the full key path

#### Scenario: Absent request is inert
- **WHEN** a manifest requests no allocation
- **THEN** the compiled policy and the generated service configuration
  are byte-identical to the same manifest before this capability existed

#### Scenario: Request without a service is rejected
- **WHEN** an allocation request names a variable but no service, and is
  not explicitly scoped to sessions
- **THEN** validation fails at layer `config` with exit code 2, stating
  that an allocation must name the service receiving it

### Requirement: Variable names are validated, not merely stored
The system SHALL reject an allocation request naming something that
cannot be an environment variable, at parse time rather than at `up`.

#### Scenario: Empty name rejected
- **WHEN** an allocation request contains an empty variable name
- **THEN** validation fails at layer `config` with exit code 2

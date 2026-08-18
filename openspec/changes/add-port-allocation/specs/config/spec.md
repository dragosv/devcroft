# config Delta Specification (add-port-allocation)

## ADDED Requirements

### Requirement: Port allocation request
The system SHALL accept a port-allocation request in `[network]`,
naming the environment variables to be allocated, validated with the
same strictness as neighbouring keys: unknown keys rejected with the
full key path, malformed values failing at layer `config` with exit
code 2. Omitting the request SHALL leave behaviour identical to a
manifest written before this capability existed.

#### Scenario: Malformed request rejected
- **WHEN** the allocation request is not a list of variable names
- **THEN** validation fails at layer `config` with exit code 2, naming
  the full key path

#### Scenario: Absent request is inert
- **WHEN** a manifest requests no allocation
- **THEN** the compiled policy and the generated service configuration
  are byte-identical to the same manifest before this capability existed

### Requirement: Variable names are validated, not merely stored
The system SHALL reject an allocation request naming something that
cannot be an environment variable, at parse time rather than at `up`.

#### Scenario: Empty name rejected
- **WHEN** an allocation request contains an empty variable name
- **THEN** validation fails at layer `config` with exit code 2

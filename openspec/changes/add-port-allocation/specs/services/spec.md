# services Delta Specification (add-port-allocation)

## ADDED Requirements

### Requirement: Generated service configuration uses the allocated port
The system SHALL substitute an allocated value into the service
configuration it generates, overriding the value the provider's own
declaration carried for that variable. The system SHALL NOT modify the
provider's manifest, and SHALL NOT rewrite a service's command string.

#### Scenario: Provider value is overridden
- **WHEN** a provider declares `PGPORT = "5432"` for a service and
  `PGPORT` is allocated
- **THEN** the generated configuration carries the allocated port, and
  the provider's own manifest on disk is unchanged

#### Scenario: Unrelated variables are untouched
- **WHEN** a service declares variables beyond the allocated one
- **THEN** those keep the values the provider declared

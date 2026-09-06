# Manifest Key Liveness

## ADDED Requirements

### Requirement: Every accepted manifest key SHALL be consumed

Every field the manifest schema accepts SHALL be read by code outside the
config layer, or SHALL be listed as deliberately inert with a stated reason.
This SHALL be checked mechanically, because the failure it prevents is invisible
to every other check: a key that parses, validates, and suggests corrections for
typos while reaching nothing.

#### Scenario: A key with no consumer fails the build

- **GIVEN** a field added to the manifest schema
- **WHEN** nothing outside the config layer reads it
- **THEN** the test suite SHALL fail, naming the key

#### Scenario: A deliberately inert key is declared, not silently allowed

- **GIVEN** a key that is accepted and ignored on purpose
- **WHEN** the check runs
- **THEN** it SHALL pass only because that key appears in a single declared
  list with a reason recorded beside it

#### Scenario: The check can itself fail

- **WHEN** a consumer of an existing key is removed
- **THEN** the check SHALL fail — established by removing one and observing it,
  not by assuming the check works

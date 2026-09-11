# services Delta Specification (fix-service-hook-ordering)

## ADDED Requirements

### Requirement: Services start after the provider's activation script
Where a provider supplies an activation script, the system SHALL run it
to completion before starting any declared service.

The script is what prepares the environment the services run in, and for
at least one supported provider it prepares the services' own state: a
devbox plugin's setup script creates the database's data directory, so a
database started before it has nothing to start against. Starting
services first makes that failure look like a broken service rather than
an ordering mistake.

This SHALL NOT be achieved by moving the services' lifetime out of the
keeper. A session tied to `up`'s connection is killed when `up` exits —
measured — so services started that way would be reaped seconds after
the sandbox came up.

#### Scenario: A service depending on hook-prepared state starts after it
- **WHEN** a provider's activation script prepares state a declared
  service requires, and the sandbox comes up
- **THEN** the script has completed before the service is started

#### Scenario: Teardown is unaffected
- **WHEN** the sandbox is torn down
- **THEN** no service process started by it remains alive, verified by
  observing process absence rather than by a stop command's exit status

#### Scenario: Providers whose services do not depend on the hook are unchanged
- **WHEN** a provider's services do not depend on anything its activation
  script does
- **THEN** they behave exactly as before this requirement existed

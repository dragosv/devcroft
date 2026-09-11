# services Delta Specification (add-devbox-services)

## ADDED Requirements

### Requirement: Declarations may come from the provider's plugins, not only from the project
A provider's service declarations MAY be authored by the provider's own
plugins rather than written by the project, and the system SHALL
supervise them the same way either case.

This is a decision, and it is recorded as one because it was made against
a stated argument. Every other provider's declarations are something the
project wrote or evaluated; devbox's come from whichever plugin a package
name activates, so someone adding a package acquires services they never
described. The project owner decided that acquiring them is what a user
adding `postgresql` wants. `docs/decisions.md` keeps the argument that
was decided against.

Where declarations are plugin-authored, the system SHALL make their
origin discoverable — a user who sees a service in `status` that they
never wrote SHALL be able to find out from devcroft's own documentation
why it is there.

#### Scenario: A plugin's services are supervised like any other
- **WHEN** a devbox project declares a package whose plugin ships service
  declarations
- **THEN** those services run inside the sandbox, supervised by the
  keeper, enumerable with their state, and reaped at teardown

#### Scenario: One plugin may contribute several services
- **WHEN** a plugin declares more than one process
- **THEN** each becomes its own service, individually enumerable

#### Scenario: Declarations are read without executing project code
- **WHEN** a project defines an activation hook with an observable side
  effect and declares a package whose plugin ships services
- **THEN** the services are known to the system, and the hook's side
  effect has not occurred when `up` returns

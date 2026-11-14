## Purpose

Close the half of the service-silence problem that `reconcile` does not
reach: a declaration lost on the way *in*, before any supervisor sees it.

## ADDED Requirements

### Requirement: A declaration that cannot be carried is refused, not dropped

Where a provider declares a service with something devcroft's declaration
vocabulary cannot represent, `up` SHALL fail at layer `provider`, naming what
could not be carried.

It SHALL NOT render a configuration that omits the unrepresentable part, and
SHALL NOT proceed with a warning. The symptom a warning permits is the one
this project already refuses one step later: a sandbox that starts, reports
healthy, and does not do what the manifest said.

#### Scenario: A provider declares a key devcroft does not carry

- **WHEN** a provider's service declaration contains a field devcroft's
  vocabulary has no place for
- **THEN** `up` fails at layer `provider`, naming the field and the service
- **AND** no sandbox state is written

#### Scenario: Everything in the declaration is carried

- **WHEN** every part of a declaration maps onto what devcroft represents
- **THEN** `up` proceeds exactly as it does today
- **AND** the check adds no observable behaviour

### Requirement: The check belongs to the provider reader

Detecting the residue SHALL be the responsibility of whichever code turns a
provider's own format into devcroft's declaration type, because only that
code knows what it saw and what it consumed.

Stated as a requirement because the failure is invisible downstream: by the
time a declaration reaches the shared machinery it is already reduced to the
fields devcroft carries, and nothing there can tell an absent field from a
discarded one. A provider reader that quietly ignores an unknown key is the
bug, and no later stage can catch it.

#### Scenario: A new provider is added

- **WHEN** a provider reader is written for a format devcroft has not seen
- **THEN** it reports keys it did not consume rather than ignoring them
- **AND** it does so through shared machinery rather than a hand-rolled check
  per provider

### Requirement: The vocabulary grows only when a provider needs it

Fields SHALL NOT be added to the declaration type because a supervisor
supports them. They are added when a provider's own documented schema
declares them.

The distinction matters because the supervisor's feature set is not a
contract with anyone: adding `depends_on` or health checks on the strength of
process-compose supporting them would invent an interface no provider has
asked for and no manifest can populate — and it would have to be supported
forever if the supervisor ever changed.

#### Scenario: The supervisor supports more than any provider declares

- **WHEN** the supervisor offers a capability no provider's schema exposes
- **THEN** the declaration vocabulary does not grow to match it
- **AND** a project wanting that capability is a request against the
  provider's schema, not against devcroft

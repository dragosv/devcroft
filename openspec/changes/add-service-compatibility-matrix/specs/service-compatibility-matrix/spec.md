# Spec Delta: service-compatibility-matrix

## ADDED Requirements

### Requirement: The catalog is enumerated from the provider, never hardcoded

The matrix SHALL derive the set of services it tests from the provider's
own module set at the version in use, and SHALL record that version
alongside the results. A list transcribed into the script becomes wrong
the first time upstream adds a module, and wrong silently — the missing
service reads as a service nobody had a problem with.

#### Scenario: A service module the enumeration finds has no row

- **WHEN** the provider's module set contains a service the run produced
  no row for
- **THEN** the run SHALL fail, naming that service
- **AND** the matrix SHALL NOT be written, because a matrix missing a row
  is read as a matrix whose subject was fine

#### Scenario: Upstream adds a module between runs

- **WHEN** the provider is upgraded and its module set grows
- **THEN** the next full run SHALL include the new service without the
  script being edited

### Requirement: Each service is measured through a real sandbox

A row SHALL be produced by declaring the service in a project, bringing
that project up under `devcroft`, and observing the supervised service —
never by reading a generated configuration, and never by evaluating the
provider alone. What is being measured is whether devcroft can run the
service, and only running it measures that.

#### Scenario: A service reaches ready

- **WHEN** the sandbox comes up and the service's readiness is satisfied
- **THEN** the row SHALL record the service as ready

#### Scenario: A service starts but never becomes ready

- **WHEN** the service is spawned and its readiness is not satisfied
  within the run's bound
- **THEN** the row SHALL distinguish this from ready, and SHALL record
  what the supervisor last reported

#### Scenario: A service declares no readiness at all

- **WHEN** the service module declares no probe
- **THEN** the row SHALL say that the outcome is *started*, and SHALL NOT
  present it as ready — devcroft did not observe readiness and may not
  claim it

### Requirement: The outcome vocabulary separates the responsible party

Every row SHALL attribute its outcome to one of: the service worked;
devcroft refused to carry something the service declared; devcroft ran it
and it failed; the service needs configuration the run did not supply; or
the host could not build it. These are five different pieces of news, and
collapsing them into pass/fail destroys the only part anyone acts on.

#### Scenario: devcroft's own translation refuses a field

- **WHEN** `up` fails at layer `provider` because the service declares
  something devcroft cannot carry
- **THEN** the row SHALL attribute the outcome to devcroft, and SHALL
  record the field by name

#### Scenario: The service needs credentials or external state

- **WHEN** the service fails because the run supplied no credentials,
  no data directory, or no external dependency it requires
- **THEN** the row SHALL be attributed to configuration, not to devcroft,
  and SHALL say what was missing

#### Scenario: The closure cannot be built on this host

- **WHEN** the service's package does not build or is unavailable for the
  host platform
- **THEN** the row SHALL record **not measured**, and SHALL NOT be
  counted among either the working or the failing services

### Requirement: A failure carries its reason

A failing row SHALL carry the evidence that produced it — the error
devcroft reported, or the service's own last output — in a form a reader
can act on without re-running the case. A bare status tells a maintainer
that something is wrong forty-two times and what is wrong zero times.

#### Scenario: Reading a failing row

- **WHEN** a maintainer reads a failing row
- **THEN** they SHALL find the failure's reason without re-running the
  service

#### Scenario: Two services fail the same way

- **WHEN** several services fail for one shared cause
- **THEN** their rows SHALL be recognisable as the same cause, so one
  fix is not mistaken for several problems

### Requirement: Cases are isolated from each other

Each service SHALL be measured in its own project directory and its own
sandbox, and that sandbox SHALL be torn down before the next case begins.
A service that leaks a process, holds a port, or leaves state behind
SHALL NOT be able to change another service's outcome.

#### Scenario: A service leaves a process running

- **WHEN** a case ends with a process the teardown did not reap
- **THEN** the run SHALL record that against the service whose case it
  was, and SHALL NOT allow it to run during any later case

#### Scenario: A case cannot be torn down

- **WHEN** teardown fails for one service
- **THEN** the run SHALL report it and SHALL NOT silently continue into a
  case whose starting state is unknown

### Requirement: The matrix states what it was measured on

The published matrix SHALL name the platform, the provider version, the
supervisor version, and the devcroft commit it was produced from. A
compatibility claim without those is a claim about an unspecified
machine on an unspecified day.

#### Scenario: Reading a matrix produced elsewhere

- **WHEN** a reader opens a matrix produced on a host unlike theirs
- **THEN** they SHALL be able to tell that from the document itself

#### Scenario: A partial run is published

- **WHEN** only a subset of services was measured
- **THEN** the matrix SHALL say so, and SHALL NOT present the unmeasured
  services as absent from the catalog

### Requirement: A subset can be measured without re-measuring everything

The run SHALL accept a named subset of services, producing the same rows
for them that a full run would. A measurement nobody can afford to repeat
stops being repeated, and a matrix that is never re-run is a matrix that
quietly becomes false.

#### Scenario: Re-measuring one service after a fix

- **WHEN** a maintainer re-runs a single service after changing devcroft
- **THEN** they SHALL get that service's row without building the other
  closures

#### Scenario: A named service is not in the catalog

- **WHEN** a requested name matches no service module
- **THEN** the run SHALL fail naming it, rather than reporting an empty
  result that reads as nothing to report

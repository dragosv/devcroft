## Purpose

Make devcroft's choice of service supervisor a named, replaceable thing
rather than an assumption spread across four files — so that the requirement
it imposes on users, *"add process-compose to your environment manifest"*,
becomes something devcroft could one day lift.

## ADDED Requirements

### Requirement: One place knows what a supervisor is

Everything specific to a particular supervisor SHALL live behind a single
abstraction: the executable it needs present, the configuration devcroft
writes for it, how it is launched, and how it is asked what is running.

Everything else about services SHALL remain supervisor-agnostic — where
artifacts live, how declared services are reconciled against reported ones,
and the vocabulary states are reported in. Those are devcroft's guarantees
and its layout decisions; a supervisor answers in them rather than defining
them.

#### Scenario: Adding a second supervisor

- **WHEN** a second supervisor is introduced
- **THEN** it is an implementation of that abstraction
- **AND** no code outside the abstraction needs to change to accommodate it

#### Scenario: Reconciliation is not the supervisor's

- **WHEN** a supervisor reports fewer services than were declared
- **THEN** the discrepancy is still detected by devcroft's own reconciliation
- **AND** a supervisor cannot suppress it by reporting differently

### Requirement: The user-visible requirement names the supervisor it comes from

Where devcroft refuses because the supervisor is absent from the resolved
environment, the message SHALL name that supervisor.

This is the one place a user meets the coupling, so it is the one place that
must not be phrased as if the tool were a law of nature. A project told to
install `process-compose` should be able to see that this is devcroft's
requirement and which component it belongs to.

#### Scenario: The supervisor is missing from the environment

- **WHEN** services are declared and the supervisor's executable is not in
  the resolved environment
- **THEN** `up` fails at layer `provider`, naming the executable it needs
- **AND** the message comes from the supervisor rather than a hardcoded string

### Requirement: Decoupling changes nothing a user can observe

Introducing the abstraction SHALL NOT change the configuration written, the
protocol spoken, the refusal produced, or any service's behaviour.

Stated as a requirement because this is a refactor of a component that talks
to a third-party binary through a document it parses: a subtly different
rendering fails at runtime, not at compile time, and "the tests still pass"
is weaker evidence than it looks when the tests skip on hosts without a
supervisor.

#### Scenario: The same project before and after

- **WHEN** a project with declared services is brought up before and after
- **THEN** the generated configuration is byte-identical
- **AND** the services behave identically

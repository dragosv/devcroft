# services Delta Specification (add-service-readiness)

## ADDED Requirements

### Requirement: A service may declare when it is ready
Where a provider's manifest declares how to tell that a service is ready
to serve, the system SHALL carry that declaration to the supervisor
rather than refusing or dropping it, and SHALL report the service as
ready only once it holds.

Readiness is distinguished from started deliberately. A service that has
been spawned is not a service that can answer, and the gap between the
two is where a dependent service fails — against a database whose socket
is not open yet, most often. A system that reports "started" as "ready"
is not merely less precise; it reports something that is not true.

The system SHALL support at least a command form (the service is ready
when a command exits zero) and an HTTP form (ready when an endpoint
answers), together with how long to wait before probing, how often, and
how many consecutive results decide the outcome.

#### Scenario: A service is not ready until its probe passes
- **WHEN** a service declares a readiness probe that does not pass
  immediately
- **THEN** the sandbox reports the service as started but not ready, and
  reports it ready only once the probe passes

#### Scenario: A declaration the supervisor cannot express is refused
- **WHEN** a readiness declaration uses a mechanism the system cannot
  translate
- **THEN** `up` fails at layer `provider` naming the service and the
  part it could not carry, rather than starting the service with a
  weaker probe or none

#### Scenario: A service declaring no readiness is unaffected
- **WHEN** a service declares no readiness probe
- **THEN** its supervisor configuration is byte-identical to what it was
  before readiness was supported

## MODIFIED Requirements

### Requirement: Services and hooks are distinct mechanisms
Services and hooks SHALL remain separate mechanisms with a stated
precedence. Services are supervised, enumerable and reaped at teardown;
hooks are one-shot, and a failing hook fails `up`. A long-lived process
started by a hook SHALL NOT be adopted as a service and remains the
user's to reap.

Where one service declares a dependency on another, and the target
declares readiness, the dependent SHALL start only once the target is
**ready** — not merely once it has been started. Where the target
declares no readiness, started remains the only condition available and
the dependency SHALL wait for that.

This strengthens what a dependency means, and only where a declaration
makes the stronger meaning available. Waiting for "started" against a
target that declared a probe would honour the dependency in name while
defeating what it was declared for.

#### Scenario: Hook-started processes are not services
- **WHEN** a hook starts a long-lived process
- **THEN** it is not listed as a service, and teardown does not claim to
  have reaped it

#### Scenario: A dependency waits for readiness where one is declared
- **WHEN** a service depends on another that declares a readiness probe
- **THEN** it starts after that probe passes, observed by the order in
  which the two actually start

#### Scenario: A dependency waits for start where no readiness exists
- **WHEN** a service depends on another that declares no readiness probe
- **THEN** it starts once the target has been started

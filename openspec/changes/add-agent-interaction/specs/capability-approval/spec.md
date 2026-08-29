# capability-approval Delta Specification (add-agent-interaction)

## Purpose

Turn a policy denial from a dead end into a question, without turning the
policy into a suggestion.

An agent that hits the boundary currently gets an errno and has to guess. This
lets it ask instead — bounded by what the manifest said may be asked, denied
unless someone says otherwise, and visible in the compiled policy like any
other rule.

## ADDED Requirements

### Requirement: Runtime capability expansion is opt-in and bounded by a declared envelope

Runtime capability requests SHALL be disabled unless the manifest enables them,
and SHALL be constrained by a declared envelope naming what may be *requested*.
A request outside the envelope SHALL be refused without reaching any approver.

The envelope is the whole difference between this feature and a hole. A channel
that can request anything, approved by a human under time pressure who sees one
path at a time, converges on "allow" — and the result is a sandbox that looks
confined and is not. Declaring the envelope moves the real decision to the
manifest, where it is reviewable, and leaves the runtime decision as the
narrower question of whether *this* request within an already-agreed boundary
is wanted now.

#### Scenario: Approval is not enabled

- **WHEN** a sandbox whose manifest does not enable capability requests hits a
  denial
- **THEN** the denial is final, exactly as today
- **AND** no approver is consulted, because none was asked for

#### Scenario: A request inside the envelope

- **WHEN** an agent requests a capability the manifest declared as requestable
- **THEN** the request reaches the approver

#### Scenario: A request outside the envelope

- **WHEN** an agent requests a capability the envelope does not cover
- **THEN** it is refused without consulting the approver
- **AND** the refusal distinguishes "outside what may be asked" from "asked and
  refused", since only the first is a manifest problem

### Requirement: Denial is the default, and silence is denial

An unanswered request SHALL be denied. An approver that is absent,
unreachable, errors, or does not respond within a bounded time SHALL result in
denial, never in a grant.

This matches the mechanism's own contract — the sandbox library's approval
backend documents that the supervisor "should apply a timeout and treat expiry
as a denial", and that backend errors are treated as denials. It is restated
here because it is the property most likely to be quietly inverted by an
implementation trying to be helpful when nobody is watching the fleet.

#### Scenario: Nobody answers

- **WHEN** a capability request receives no decision within the bounded time
- **THEN** it is denied
- **AND** the agent is told it was denied for want of an answer, which is a
  different fact from being refused on the merits

#### Scenario: No approver is configured

- **WHEN** capability requests are enabled but no approver is available
- **THEN** every request is denied
- **AND** the sandbox does not start in, or drift into, a state where requests
  are granted by default

### Requirement: The approval envelope is visible in the compiled policy

`policy --render` SHALL show that runtime capability expansion is enabled and
what its envelope covers.

devcroft's standing invariant is that nothing reaches the backend which
`--render` cannot show. A mechanism able to widen the enforced policy *after*
compilation is the sharpest possible test of that rule: a reader comparing two
manifests must be able to see that one of them can grow at runtime and the
other cannot.

#### Scenario: Rendering a policy with approval enabled

- **WHEN** the operator renders a policy whose manifest enables capability
  requests
- **THEN** the envelope appears as its own entry with its origin
- **AND** it is distinguishable from the grants that are in force unconditionally

#### Scenario: Rendering a policy without it

- **WHEN** the operator renders a policy that does not enable requests
- **THEN** nothing suggests runtime expansion is possible

### Requirement: Every request and decision is recorded

Each request, its decision, and the reason SHALL be recorded durably enough to
answer "what did this agent ask for, and what did it get" after the fact.

Without this the feature is unauditable in the specific way that matters: a
sandbox's *effective* policy becomes the compiled policy plus an unknown series
of runtime grants, and `policy --render` would describe only the first half.

#### Scenario: Reviewing an agent's requests

- **WHEN** an operator reviews a sandbox after it has run
- **THEN** each capability request, its decision and the reason are available
- **AND** a granted request is distinguishable from one that was never made

#### Scenario: A grant is made

- **WHEN** a capability request is granted
- **THEN** the record shows what was granted, to which sandbox, and on whose
  decision

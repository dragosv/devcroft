# agent-attention Delta Specification (add-agent-interaction)

## Purpose

Let a sandbox say *a human is needed here*, and let an operator find which one
without attaching to each in turn.

The scope is deliberately narrow: devcroft carries a state and a message. It
does not decide what warrants attention, does not classify the message, and
does not deliver it anywhere. That is the agent's business and the operator's
respectively; devcroft's job is that the fact is not lost.

## ADDED Requirements

### Requirement: A sandbox can be marked as needing attention, from inside it

A process inside a sandbox SHALL be able to mark that sandbox as needing
attention, supplying a message, and SHALL be able to clear that state again.
Setting it SHALL NOT require any capability the sandbox does not already have.

Raising attention SHALL NOT block the process that raises it, and SHALL NOT
alter the sandbox's execution. An agent that flags a question and then exits,
keeps working, or waits on its own input is equally valid — devcroft records
the flag, it does not impose a control flow on the agent.

#### Scenario: An agent needs a judgment

- **WHEN** an agent inside a sandbox raises attention with a message
- **THEN** the sandbox is recorded as needing attention, with that message
- **AND** the agent continues or exits on its own terms; nothing about its
  execution changes

#### Scenario: The agent resolves it itself

- **WHEN** an agent that raised attention later clears it
- **THEN** the sandbox is no longer reported as needing attention
- **AND** no operator action was required for that transition

#### Scenario: Raising attention requires no extra grant

- **WHEN** a sandbox under a deny-by-default policy raises attention
- **THEN** it succeeds
- **AND** no manifest change was needed to permit it, because a mechanism that
  requires widening the policy to report a problem would go unused in exactly
  the confined sandboxes that need it most

### Requirement: Attention is reported in the listing, not only per sandbox

`ps` SHALL report which sandboxes need attention. `status` SHALL report the
state and its message for one sandbox.

Reporting it only in `status` would satisfy the letter and miss the point: with
N sandboxes, "which one is blocked" is the question, and answering it by
running `status` N times is the search this requirement exists to remove.

#### Scenario: One agent in a fleet is blocked

- **WHEN** several sandboxes are running and one needs attention
- **THEN** a single `ps` identifies which one
- **AND** the operator does not have to attach to, or query, the others

#### Scenario: Reading the message

- **WHEN** the operator inspects a sandbox needing attention
- **THEN** the message the agent supplied is shown verbatim
- **AND** it is not summarised, truncated to uselessness, or interpreted

### Requirement: Attention is distinguishable from unhealthy

Needing attention SHALL be reported as its own state, distinct from a keeper
that is unhealthy, a service that failed, or a sandbox that is not running.

These have opposite meanings for the operator and opposite fixes. An agent
waiting on a decision is *working correctly* — the sandbox is healthy, the
services are up, and nothing is broken. Folding it into a failure state would
train the operator to treat a normal, expected condition as an error, which is
how real errors stop being noticed.

#### Scenario: A healthy sandbox needing attention

- **WHEN** a sandbox's keeper is healthy and its services are running, and it
  needs attention
- **THEN** it is reported as healthy *and* needing attention
- **AND** neither fact is presented as contradicting the other

#### Scenario: An unhealthy sandbox

- **WHEN** a sandbox's keeper has died
- **THEN** that is reported as the failure it is, not as needing attention

### Requirement: Attention survives what it must, and no more

The state SHALL persist across the raising process exiting, so that an agent
which flags a question and terminates is still reported. It SHALL NOT survive
the sandbox being torn down and recreated.

An agent's most common shape is: work, reach a decision point, report, exit.
A state that vanished with the process would miss precisely that case. But a
state that outlived `down`/`up` would report a question about work that no
longer exists.

#### Scenario: The agent exits after flagging

- **WHEN** an agent raises attention and then exits
- **THEN** the sandbox is still reported as needing attention

#### Scenario: The sandbox is recreated

- **WHEN** a sandbox that needed attention is brought down and up again
- **THEN** it does not start in the attention state

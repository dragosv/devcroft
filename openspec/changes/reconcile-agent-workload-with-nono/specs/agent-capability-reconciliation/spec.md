## Purpose

Stop a proposal from silently disagreeing with what its dependencies offer,
by requiring the disagreement to be resolved and recorded rather than
discovered later.

## ADDED Requirements

### Requirement: A capability the backend offers is adopted or rejected with a property

For each capability the backend-capability matrix records as unadopted and
that bears on the work being proposed, the proposal SHALL record one of:
adopted, rejected, or deferred — and a rejection SHALL name the property that
fails rather than a preference.

A property is something a later reader can check and, if it stops being true,
act on: "it does not enforce", "it lives in a crate we do not link", "it
duplicates a mechanism we already have". "We decided not to" is not one.

#### Scenario: A capability that does not do what its name suggests

- **WHEN** a capability is considered and turns out not to provide the
  behaviour its name implies
- **THEN** it is rejected naming that, and the matrix keeps reporting it
  unadopted
- **AND** it is not adopted merely to make the matrix look complete

#### Scenario: A capability whose cost lives elsewhere

- **WHEN** a capability is provided by a crate the project does not currently
  depend on
- **THEN** the decision names that dependency and its cost
- **AND** adopting it is that dependency's decision, not a side effect of the
  change that wanted the capability

### Requirement: A deferral carries the condition that ends it

Where the answer depends on something not yet true, the proposal SHALL record
what would make it true.

Without that, a deferral is indistinguishable from a rejection whose reason
was forgotten, and gets re-argued from scratch instead of revisited. This
project already applies the rule to its own rejections — `docs/decisions.md`
is written to be falsifiable — and a dependency decision is no different.

#### Scenario: The condition becomes true

- **WHEN** the circumstance a deferral named comes about
- **THEN** the decision is revisited rather than re-derived
- **AND** the record says what changed

### Requirement: A proposal that predates a dependency is reconciled before it is built

Where a proposal was written before a dependency the project has since
adopted, it SHALL be reconciled against what that dependency offers before
implementation starts.

Stated because the failure is silent and the cost lands late: the proposal
still reads as considered, so the reimplementation it leads to is discovered
after the code exists, not before. Chronology is checkable — creation dates
are in version control — so this is a question that can always be asked
rather than a judgement about whether an author was thorough.

#### Scenario: Implementation is about to begin on an older proposal

- **WHEN** work starts on a proposal older than a dependency it would
  naturally use
- **THEN** the reconciliation happens first
- **AND** its outcome is recorded in the proposal rather than only in the
  implementation

# backend-capabilities Delta Specification (add-backend-capabilities)

## Purpose

One declared, machine-readable answer to "what does devcroft actually
enforce?" — replacing the same claim maintained by hand in five prose
locations, which has drifted every time the code moved.

## ADDED Requirements

### Requirement: Capabilities are declared as data, not prose

The system SHALL maintain a machine-readable declaration of its capabilities.
Each entry SHALL record what the capability is, its status per platform, and
the evidence establishing that status.

Documentation SHALL defer to this declaration rather than restating it. Where a
document and the declaration disagree, the document is wrong — a rule
`docs/threat-model.md` already states and which this requirement makes
actionable by giving it something to point at.

#### Scenario: A capability's status changes

- **WHEN** devcroft gains, loses, or changes the strength of a capability
- **THEN** the declaration is updated as part of that change
- **AND** no separate prose edit is required for the claim to be correct,
  because the prose does not restate it

#### Scenario: Prose contradicts the declaration

- **WHEN** a document asserts a capability claim that the declaration
  contradicts
- **THEN** the document is treated as the defect
- **AND** the resolution is to remove the restatement, not to update it in place

### Requirement: Status uses a closed vocabulary that distinguishes unmeasured from unsupported

A capability's status on a platform SHALL be exactly one of: `enforced`,
`enforced-with-named-degradation`, `unsupported`, `not-adopted`, or
`unverified`. Free-text status SHALL NOT be permitted.

The vocabulary exists to force three distinctions prose keeps collapsing:

- **`not-adopted` versus `unsupported`.** "devcroft does not use this" and "this
  platform cannot do this" are different facts with different fixes — one is a
  scope decision, the other is a constraint.
- **`unverified` versus `enforced`.** A capability believed to work and one
  measured to work are not the same claim, and this project has shipped the
  former as the latter more than once.
- **`enforced-with-named-degradation` versus `enforced`.** A capability that
  works differently on one platform must say so where the claim is made, not in
  a footnote elsewhere.

#### Scenario: A capability works on one platform and is unmeasured on another

- **WHEN** a capability is measured on Linux and has never been run on macOS
- **THEN** it is `enforced` on Linux and `unverified` on macOS
- **AND** it is not reported as enforced on both, nor as unsupported on macOS

#### Scenario: The library offers something devcroft does not use

- **WHEN** the sandbox library provides a capability devcroft does not
  configure
- **THEN** it is recorded as `not-adopted`, not omitted
- **AND** the declaration therefore shows the gap between what is available and
  what is used, which is the axis that remains once there is a single backend

### Requirement: Every claim names its evidence

An entry SHALL cite what established its status — a test, a live measurement,
or an upstream guarantee. A capability whose status rests on inference SHALL be
`unverified` regardless of how reasonable the inference is.

This is the requirement that makes the rest worth having. Without it the matrix
becomes the same unverified prose in a new format, and this project's recurring
defect is precisely a claim that was reasonable, unmeasured, and wrong.

#### Scenario: An entry claims enforcement

- **WHEN** an entry is marked `enforced`
- **THEN** it names the test or measurement that demonstrates it
- **AND** a reader can re-run that evidence

#### Scenario: An entry rests on a plausible argument

- **WHEN** a capability is believed to hold because the mechanism suggests it
  should, but nothing has exercised it
- **THEN** it is `unverified`, and the argument is recorded as the reason to
  check rather than as the basis of a claim

### Requirement: Declared capabilities are reported against the actual host

`doctor` SHALL report the declared capabilities alongside what this host can
provide, so that "devcroft supports this" and "this machine can do this" are
distinguishable without inference.

#### Scenario: A host lacks something devcroft enforces elsewhere

- **WHEN** a capability is `enforced` in the declaration but unavailable on this
  host
- **THEN** `doctor` reports both facts and the difference between them
- **AND** the user is not left to deduce whether the gap is devcroft's or their
  machine's

#### Scenario: An unadopted capability

- **WHEN** a capability is `not-adopted`
- **THEN** `doctor` does not probe the host for it
- **AND** it is not reported as a host deficiency, since the host is not the
  reason it is absent

### Requirement: A change that alters a capability updates the declaration

A change that adds, removes, or alters the strength of a capability SHALL
update the declaration in the same change. The declaration SHALL NOT be
maintained as a separate follow-up.

#### Scenario: A change ships enforcement for something previously declared

- **WHEN** a change makes a previously `not-adopted` capability enforced
- **THEN** that change updates the entry and its evidence
- **AND** the declaration is never a lagging record of what shipped earlier

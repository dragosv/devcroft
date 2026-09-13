# backend-capabilities Delta Specification (add-not-applicable-status)

## MODIFIED Requirements

### Requirement: Status uses a closed vocabulary that distinguishes unmeasured from unsupported

A capability's status on a platform SHALL be exactly one of: `enforced`,
`enforced-with-named-degradation`, `unsupported`, `not-applicable`,
`not-adopted`, or `unverified`. Free-text status SHALL NOT be permitted.

The vocabulary exists to force four distinctions prose keeps collapsing:

- **`not-adopted` versus `unsupported`.** "devcroft does not use this" and
  "this platform cannot do this" are different facts with different fixes —
  one is a scope decision, the other is a constraint.
- **`unverified` versus `enforced`.** A capability believed to work and one
  measured to work are not the same claim, and this project has shipped the
  former as the latter more than once.
- **`enforced-with-named-degradation` versus `enforced`.** A capability that
  works differently on one platform must say so where the claim is made, not
  in a footnote elsewhere.
- **`not-applicable` versus `unsupported`.** "this platform cannot protect
  you from that" and "that does not exist on this platform" are opposite
  news wearing one word. The first is a loss the reader should weigh; the
  second is nothing to weigh at all. Collapsing them makes a platform look
  weaker than it is, which is the same failure as making it look stronger —
  the matrix exists to be accurate in both directions.

`not-applicable` SHALL mean that the capability's *subject* does not exist
on that platform, and SHALL NOT be used for a capability that exists and is
unenforced, unmeasured, or unimplemented. Those have words already.

#### Scenario: A capability works on one platform and is unmeasured on another

- **WHEN** a capability is measured on Linux and has never been run on macOS
- **THEN** it is `enforced` on Linux and `unverified` on macOS
- **AND** it is not reported as enforced on both, nor as unsupported on macOS

#### Scenario: The thing a capability protects against does not exist on a platform

- **WHEN** a capability's subject is absent from a platform — measured, not
  assumed — so there is nothing for the sandbox to reach and nothing to scope
- **THEN** its status on that platform is `not-applicable`
- **AND** it is not reported as `unsupported`, which would tell the reader
  they are exposed to something that is not there
- **AND** it is not reported as `enforced`, which would claim an enforcement
  nothing performs

#### Scenario: A genuine platform loss keeps its word

- **WHEN** a platform cannot deliver a capability whose subject *does* exist
  there, and the reader is materially worse off as a result
- **THEN** its status stays `unsupported`, regardless of how inconvenient
  that is to report

#### Scenario: Evidence is required for the new value too

- **WHEN** an entry claims `not-applicable`
- **THEN** it cites what established the subject's absence, to the same
  standard every other status is held to — absence is a measurable claim,
  and asserting it from familiarity with a platform is the inference this
  requirement's second distinction exists to refuse

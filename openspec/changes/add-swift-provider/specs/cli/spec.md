# cli Delta Specification (add-swift-provider)

## ADDED Requirements

### Requirement: Guarantee tier is user-visible
The system SHALL print the resolved environment's guarantee tier once at
`up` and on every `status`, so that two sandboxes backed by different
guarantees can be told apart without reading documentation.

For the `artifact` tier the line SHALL name what the tier does not
promise — that the runtime links against host libraries — rather than
printing the bare word.

#### Scenario: status names the tier
- **WHEN** `devcroft status` runs for a sandbox
- **THEN** the output contains a line naming the provider's guarantee
  tier

#### Scenario: artifact tier is qualified, not merely labelled
- **WHEN** the provider is `swift`
- **THEN** the tier line states that the runtime is host-linked

#### Scenario: closure sandboxes are unchanged in substance
- **WHEN** the provider is `flox`, `nix`, or `devbox`
- **THEN** the tier line reports `closure`

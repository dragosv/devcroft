## ADDED Requirements

### Requirement: hermit classification is measured
The system SHALL classify `env.provider = "hermit"` according to this change's
qualification. If admitted, it SHALL be accepted as an artifact-tier provider.
If rejected, it SHALL fail at layer `config`, exit code 2, with a reason
naming the failing property, and SHALL NOT be described anywhere as qualified
or planned.

#### Scenario: Rejected by qualification
- **WHEN** the qualification rejects hermit and the manifest declares `provider = "hermit"`
- **THEN** validation fails at layer `config` and the message names the failing criterion

#### Scenario: Admitted by qualification
- **WHEN** the qualification admits a profile and the manifest declares `provider = "hermit"`
- **THEN** validation succeeds and `status` reports tier `artifact`

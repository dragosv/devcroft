# lifecycle Specification

## Purpose

Surface the mise provider's guarantee tier in `status`, alongside the
keeper health/uptime/staleness `status` already reports (lifecycle spec's
"Status and logs" requirement) — this belongs here, not in `cli`, since
`status`'s content is a lifecycle-capability concern; `cli` only owns the
command surface itself. Delta against `openspec/specs/lifecycle/`
(currently empty — no change has archived yet), so this is additive, not a
modification of an existing requirement.

## ADDED Requirements

### Requirement: Guarantee tier surfaced in status
The system SHALL include the active guarantee tier (`closure` or
`artifact`) in `status` output, derived from the sandbox's provider, so the
weaker artifact-integrity guarantee mise provides is never presented as
equivalent to flox's closure guarantee.

#### Scenario: Status with mise
- **WHEN** running `status` on a sandbox using the mise provider
- **THEN** the output includes `guarantee: artifact`

#### Scenario: Status with flox
- **WHEN** running `status` on a sandbox using the flox provider
- **THEN** the output includes `guarantee: closure`

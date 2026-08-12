# policy Specification

## Purpose

Grant sandboxes using the mise provider a read-only, origin-annotated
mount of the devcroft-owned mise data dir — the policy-compilation half of
the mise provider's store sharing (the env-provider spec covers
resolution; this covers the resulting grant). Delta against
`openspec/specs/policy/` (currently empty — no change has archived yet),
so this is additive, not a modification of an existing requirement.

## ADDED Requirements

### Requirement: mise data dir read-only grant
The system SHALL mount the devcroft-owned `MISE_DATA_DIR` read-only for
sandboxes using the mise provider, with origin `provider:mise`, the same
way flox's store path is granted automatically without a manifest entry.

#### Scenario: Mise data dir mount
- **WHEN** using the mise provider
- **THEN** the sandbox gets a read-only grant to the shared cache, and
  `policy --render` shows it with origin `provider:mise`

#### Scenario: No write access, even for install
- **WHEN** a sandboxed session attempts to write to `MISE_DATA_DIR`
- **THEN** the kernel denies it — devcroft only ever installs into that
  directory host-side at `up`, never from inside a sandbox

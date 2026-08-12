# config Specification

## Purpose

Extend the manifest schema with the isolation-tier intent this change
introduces. Delta against `openspec/specs/config/` (currently empty — no
change has archived yet), so this is additive, not a modification of an
existing requirement.

## ADDED Requirements

### Requirement: Isolation tier selection
The system SHALL accept an optional `[sandbox].isolation` key with values
`"process"` (default) or `"hardened"`. The value is an intent, resolved to
a concrete backend per host; the manifest never names a backend directly.

#### Scenario: Isolation intent
- **WHEN** the manifest sets `isolation = "hardened"`
- **THEN** the system resolves the hardened tier to whichever supported
  backend (gVisor, LiteBox) is available on this host

#### Scenario: Default is unchanged
- **WHEN** `[sandbox].isolation` is absent
- **THEN** the system behaves exactly as the `process` tier does today —
  this key is additive and never changes default behavior

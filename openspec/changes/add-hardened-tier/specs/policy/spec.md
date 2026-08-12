# policy Specification

## Purpose

Target policy compilation at the hardened backend's own policy model
instead of nono's, without changing the manifest schema. Delta against
`openspec/specs/policy/` (currently empty — no change has archived yet),
so this is additive, not a modification of an existing requirement.

## ADDED Requirements

### Requirement: Hardened backend policy target
When `isolation = "hardened"`, the system SHALL compile the same manifest
into the resolved hardened backend's policy model (e.g. gVisor's runsc
config plus a Landlock profile for defense in depth) instead of nono's
profile format. The manifest, and every rule's origin annotation
(`manifest:<key>` / `provider:<name>` / `baseline`), SHALL be unaffected by
which tier compiles it.

#### Scenario: Hardened policy format
- **WHEN** the tier is hardened
- **THEN** the compiled policy matches the backend's expected format, and
  `policy --render` shows the same origin-annotated rules a `process`-tier
  render of the same manifest would show

#### Scenario: Backend selection is not user-visible in the manifest
- **WHEN** two hosts both resolve `isolation = "hardened"` to different
  concrete backends (one gVisor, one LiteBox)
- **THEN** the manifest and its compiled policy semantics are identical;
  only `status` names which concrete backend is in use

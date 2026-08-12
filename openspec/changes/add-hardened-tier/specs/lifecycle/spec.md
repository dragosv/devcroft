# lifecycle Specification

## Purpose

Make the keeper conditional on tier and backend: the `process` tier and any
hardened backend without a native exec-into primitive still need the
spawn-server keeper; a hardened backend that provides one (e.g. `runsc
exec`) does not. Delta against `openspec/specs/lifecycle/` (currently
empty — no change has archived yet), so this is additive, not a
modification of an existing requirement.

## ADDED Requirements

### Requirement: Keeper conditionally required
The system SHALL use the spawn-server keeper for the `process` tier and for
any hardened backend lacking a native exec-into primitive. For a hardened
backend that provides one, the system SHALL use that primitive directly
instead of running a keeper. Session semantics (exec, shell, signals, SSH)
MUST be identical whether or not a keeper is in the path.

#### Scenario: Native exec primitive
- **WHEN** using a hardened backend with native exec (e.g. `runsc exec`)
- **THEN** no spawn-server keeper is run, and sessions still behave
  identically to the `process` tier from the user's perspective

#### Scenario: Hardened backend without native exec
- **WHEN** using a hardened backend that has no exec-into primitive
- **THEN** the keeper runs inside that backend exactly as it does for the
  `process` tier

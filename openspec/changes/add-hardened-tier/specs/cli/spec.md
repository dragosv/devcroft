# cli Specification

## Purpose

Extend `doctor` with hardened-tier diagnostics. Delta against
`openspec/specs/cli/` (currently empty — no change has archived yet), so
this is additive, not a modification of an existing requirement.

## ADDED Requirements

### Requirement: doctor reports hardened-tier availability
The system SHALL report hardened-tier availability in `doctor` output:
whether a supported backend (gVisor's `runsc`, LiteBox) is present, whether
the host provides the platform primitive each backend needs (for gVisor:
systrap by default, or KVM when `/dev/kvm` is accessible — ptrace is
deprecated upstream and not targeted), and kernel support. Each failure
names its fix, consistent with every other `doctor` check.

#### Scenario: Check hardened availability
- **WHEN** `doctor` is run
- **THEN** it checks for `runsc` and systrap/KVM support and reports
  `[PASS]`/`[FAIL]` for hardened-tier readiness alongside the existing
  backend/provider/ssh-config checks

#### Scenario: Hardened unavailable on this host
- **WHEN** no hardened backend is available (e.g. macOS)
- **THEN** `doctor` reports `[FAIL] hardened-tier: <reason>`, naming the
  platform limitation rather than a missing binary, since on macOS this is
  permanent, not fixable by installing something

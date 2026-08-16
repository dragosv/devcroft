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
deprecated upstream and not targeted), and kernel support. The hardened
tier is opt-in (`[sandbox].isolation` defaults to `process`), the same
posture the `nix` provider's absence already has in this same command, so
its unavailability alone SHALL be `[WARN]`, not `[FAIL]` — a host with no
hardened backend at all is still fully usable for every `process`-tier
project. `[FAIL]` is reserved for a backend that is present but broken.
Each failure or warning names its fix, consistent with every other
`doctor` check.

#### Scenario: Check hardened availability
- **WHEN** `doctor` is run
- **THEN** it checks for `runsc` and systrap/KVM support and reports
  `[PASS]`/`[WARN]`/`[FAIL]` for hardened-tier readiness alongside the
  existing backend/provider/ssh-config checks

#### Scenario: Hardened unavailable on this host
- **WHEN** no hardened backend is available (e.g. macOS, or Linux without
  `runsc` installed)
- **THEN** `doctor` reports `[WARN] hardened-tier: <reason>`, naming the
  platform limitation or missing binary and noting it is only needed for
  `isolation = "hardened"` projects

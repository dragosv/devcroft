# cli Delta Specification (add-gvisor-backend)

## Purpose

gVisor-specific `doctor` diagnostics — additive alongside
`add-hardened-tier`'s backend-generic "hardened-tier availability"
requirement, which stays as written. Delta against `openspec/specs/cli/`
(currently empty — no change has archived yet).

## ADDED Requirements

### Requirement: doctor gVisor diagnostics
When `runsc` is present, `doctor` SHALL report its version and whether it
falls in devcroft's tested range, which platform would be selected
(systrap or KVM) and why, and SHALL probe that the selected platform
actually works (a trivial `runsc do`-style smoke check or equivalent)
rather than inferring from binary presence alone. Absence of `runsc` on
Linux SHALL be `[WARN]` (only needed for `isolation = "hardened"`
projects, mirroring how the nix provider's absence is treated); a present
but unusable runsc SHALL be `[FAIL]` with the fix named. On macOS the
hardened tier SHALL be reported as a permanent platform limitation, not
a missing binary.

#### Scenario: runsc present and usable
- **WHEN** `doctor` runs on Linux with a working runsc and no `/dev/kvm`
- **THEN** it reports `[PASS]` naming the version and
  `platform: systrap`

#### Scenario: runsc present but platform unusable
- **WHEN** `runsc` is installed but its smoke probe fails (e.g. kernel
  too old for systrap, `/dev/kvm` present but inaccessible)
- **THEN** `doctor` reports `[FAIL]` naming the failing platform and the
  concrete fix, not just "runsc found"

#### Scenario: runsc absent on Linux
- **WHEN** `doctor` runs on Linux without runsc
- **THEN** it reports `[WARN]` noting runsc is only needed for
  `isolation = "hardened"` projects, and where to get it

#### Scenario: macOS
- **WHEN** `doctor` runs on macOS
- **THEN** the hardened-tier line names the platform limitation as
  permanent rather than suggesting an install

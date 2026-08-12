# env-provider Specification

## Purpose

Add `mise` as a second environment provider, under the `artifact`
guarantee tier introduced by this change (as opposed to `flox`'s
`closure` tier) — see `docs/decisions.md` §1's six-criterion provider
test. Delta against `openspec/specs/env-provider/` (currently empty — no
change has archived yet), so this is additive, not a modification of an
existing requirement.

## ADDED Requirements

### Requirement: mise provider (artifact tier)
The system SHALL support `provider = "mise"` under the `artifact`
guarantee tier. `up` SHALL enforce locked mode (`MISE_LOCKED=1`) so every
resolved tool has a pinned, checksummed URL for the current platform;
resolution and installation run host-side at `up`, before restrictions,
identically to the fixed composition order flox already uses.

#### Scenario: Valid mise lock
- **WHEN** a valid `mise.lock` exists and covers the current platform
- **THEN** devcroft activates the environment in locked mode and the
  sandbox comes up with zero network access required from inside

#### Scenario: Missing or incomplete lock
- **WHEN** `mise.lock` is absent, or does not cover the current platform
- **THEN** `up` fails at layer `provider` with exit code 3 and the hint
  `mise lock`

#### Scenario: Store paths become readable
- **WHEN** provider is `mise`
- **THEN** the compiled policy includes a read-only grant to the
  devcroft-owned `MISE_DATA_DIR`, annotated `provider:mise`, without the
  user declaring it — mirroring how flox's store path is granted
  automatically

### Requirement: Degraded tool coverage is surfaced
The system SHALL warn once at `up`, by name, for any tool whose backend
provides only partial lock support (e.g. checksum-only, no provenance),
rather than silently treating it as equivalent to a fully-verified tool.

#### Scenario: Partial backend coverage
- **WHEN** a tool in `mise.lock` resolves through a backend that lacks full
  provenance data
- **THEN** `up` prints one warning naming the tool and the gap, and
  proceeds

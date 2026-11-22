# Change: add-hermit-provider

## Why

devcroft describes hermit as qualified. `src/provider/validate.rs` refuses it as
"a qualified provider (artifact tier) but not yet scheduled",
`openspec/config.yaml` lists it under "qualified but unscheduled", the README
says it "passes devcroft's six-criterion provider test on paper", and
`add-mvp-core`'s env-provider spec uses it as the example of a planned
provider. That classification predates `fix-provisioning-hooks`, which
tightened criterion 4, and nothing in the repository records a measurement
behind it.

Hermit's own documentation, read on 2026-11-22, puts two criteria in doubt:

- **Criterion 2 (restorable lockfile).** Package definitions (URLs and
  checksums) live in manifest sources that "Hermit syncs … every 24 hours
  from HEAD". The documented pin is a `#<tag>` suffix on a git source. A
  project's recorded version does not fix what that version's manifest says.
- **Criterion 4 (capturable activation without project code).** Manifests
  declare triggers on `unpack`, `install`, `uninstall`, `activate` and `exec`
  ("will run for _every_ execution"), with actions that include running
  commands. A project can add its own manifest source (`env:///…`), which
  makes those triggers project code.

This change settles the classification by measurement. It is **not
scheduled**: it starts when the owner records demand, or earlier if the owner
wants the existing claim corrected without waiting.

## What Changes

- Group 0 re-qualifies hermit against all six criteria on a pinned hermit
  release. The outcome is one of two, and both complete the change:
  - **Admitted (narrow profile):** sources pinned to an immutable revision,
    every selected package with a checksum, and no trigger that runs a
    command. `env.provider = "hermit"` becomes an artifact-tier provider.
  - **Rejected:** hermit moves from "not yet supported" to a rejection naming
    the failing property, and every place that calls it qualified is
    corrected.
- If admitted, the provider follows the mise/pixi contract: devcroft verifies
  inputs before any subprocess, runs hermit against a derived configuration in
  an allowlisted environment with its state inside a devcroft-owned object,
  captures `hermit env` output as data, and stores immutable objects through
  `provider::artifacts`.
- Packages whose manifests declare `run` actions on any trigger, or any `exec`
  trigger, are refused by package. Local manifest sources (`env:///`,
  `file://`) are refused by source.

## Capabilities

### New Capabilities

None. This extends the existing provider contract.

### Modified Capabilities

- `config`: `hermit` either accepted or rejected with a measured reason; the
  "planned provider" example in `add-mvp-core` changes either way.
- `env-provider`: pinned sources, checksum-complete selection, refusal of
  command triggers, data-only capture.
- `policy`: attributed artifact and host-runtime grants.

The main spec tree is empty, so these are provider-scoped **ADDED**
requirements.

## Impact

- Either outcome edits `src/provider/validate.rs` (the `NOT_YET_SUPPORTED`
  entry and its unit test), the "Planned provider rejected in MVP" scenario in
  `openspec/changes/add-mvp-core/specs/env-provider/spec.md`,
  `openspec/config.yaml`, `README.md` (the "hermit is a different answer"
  paragraph) and `docs/decisions.md` §1 (the tier list names hermit).
- If admitted: `src/provider/hermit.rs`, `src/provider/mod.rs`,
  `src/bin/devcroft.rs`, `tests/hermit_provider_e2e.rs`,
  `samples/hermit-sample`.
- Hermit is binary-only and "deliberately not in the business of installing
  libraries". It is artifact tier, with host-runtime grants declared and
  rendered.

See [design.md](design.md) for what is documented, what must be measured, and
the stop criteria.

## Why

The two 2026-11-05 explorations propose a Linux VM per sandbox, but neither
contains a devcroft boot or comparative measurement. Selecting a VMM now would
turn unverified integration assumptions into a product dependency.

## What Changes

- Build two disposable experimental adapters: pinned hvi and libkrun **2.0**.
- Exercise identical guest artifacts per architecture in both a declared
  closure environment and an adaptable local environment.
- Gate comparison on confinement before guest execution, private state,
  controlled host channels, real keeper/SSH/services, and crash recovery.
- Measure cold preparation, warm reuse and 1/4/8-instance workloads with raw
  results, failed and unavailable rows visible.
- Produce an adoption decision that may select neither candidate. No production
  backend, public default change, or stronger isolation claim ships here.

## Capabilities

### New Capabilities

- `microvm-evaluation`: reproducible experiment, correctness gates, threat probes,
  comparable measurements and evidence-based selection.

### Modified Capabilities

None. This is an experimental capability. The repository currently has no
synced `openspec/specs/`; existing process-runtime deltas remain in force.

## Impact

Future experimental helpers, guest fixtures, kernel configuration and benchmark
scripts; isolated from the default Rust dependency/build path. Reuses keeper,
SSH and service code where demonstrated, without prematurely generalizing
`SessionBackend`. Feeds `add-vm-project-runtime`.

Sources and cross-change triage: [adversarial review](adversarial-review.md).
No benchmark or new upstream-source verification is claimed by this proposal.

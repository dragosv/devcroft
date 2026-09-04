## Why

**A service declaration devcroft cannot represent is dropped silently.**
`ServiceDecl` carries five fields — `name`, `command`, `vars`, `is_daemon`,
`shutdown_command` — and they are flox's schema, not a neutral vocabulary. A
provider declaring anything else (service dependencies, health checks, a
restart policy, socket activation) has that information discarded at
translation: `render_config` writes a valid process-compose config, the
services start, and nobody learns a declaration was ignored.

The `services` spec already forbids this class of silence one step later —
"the live answer can only describe services the supervisor accepted, so on
its own it reports a missing service as simply nothing at all" is exactly
why `reconcile` exists. The same failure one step *earlier*, at translation,
is uncovered.

This is a live gap, not a hypothetical one: it is reachable the moment any
provider's schema grows past flox's, and `add-devenv-provider` is already
open with "whether devenv's `processes` are readable as a contract" listed
as unmeasured.

## What Changes

- **NEW** `service-translation-fidelity`: a declaration devcroft cannot
  represent SHALL fail at layer `provider`, naming what it could not carry —
  the same shape `up` already uses when `process-compose` is missing from the
  resolved environment. Never a silent drop.
- **Recorded, deliberately not built: the supervisor seam.** devcroft is
  coupled to process-compose at exactly four points, and this change names
  them so a future provider with a different supervisor has a starting point
  instead of a re-derivation. It does **not** build the abstraction — see
  Non-Goals for why that would be speculative today.

## Capabilities

### New Capabilities

- `service-translation-fidelity`: what happens to a declaration devcroft
  cannot express, and where the boundary between "can carry" and "must
  refuse" is drawn.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs. The `services` capability
  this pressures lives in the unarchived `add-flox-services`, and this change
  adds a requirement adjacent to its `reconcile` guarantee rather than
  altering it.)

## Impact

- **Affected code**: `src/provider/mod.rs` (`ServiceDecl`),
  `src/services/mod.rs` (`render_config`), `src/lifecycle/up.rs`
  (`prepare_services`, which already refuses at layer `provider` for the
  missing-binary case and is the natural home for this one).
- **Measured, and it reframes the problem this change started from.** The
  question that prompted it was "what if a provider's services are based on
  something other than process-compose". Checked: **they are not, today.**
  `add-devenv-provider` records that devenv's `processes` are
  process-compose-backed — the same supervisor — and flox uses it internally
  too. process-compose is the Nix-adjacent ecosystem's common denominator,
  which makes the coupling less arbitrary than it looks and a supervisor
  abstraction premature.
- **Also measured: no crate supplies a supervisor.** `duct`, `subprocess`,
  `command-group`, `procfile` are building blocks; `supervisor` v0.1.0 is a
  placeholder with one dependency. Nothing offers "supervise N processes and
  expose their status", which is itself informative — that is an application
  concern, which is why process-compose is a binary everyone shells out to
  rather than a crate everyone links. A minimal second supervisor would be
  devcroft's own ~150 lines on `std`, needing no new dependency.

## Non-Goals

- **Not a `Supervisor` abstraction.** Its only second implementer today would
  be a test double, which is the shape this project rejects elsewhere — the
  same reasoning that dropped `up_with_resolution` as "a second entry point
  whose only distinction is covering less of the real path". The four
  coupling points are recorded in design.md so that when a provider with a
  genuinely different supervisor appears, the answer is a starting point
  rather than a re-derivation.
- **Not widening `ServiceDecl` speculatively.** Adding `depends_on` or
  health checks because process-compose supports them would be inventing a
  contract no provider has asked for. The rule is refuse-what-you-cannot-
  carry; growing the vocabulary happens when a provider's documented schema
  requires it, with that provider's change.
- **Not a change to what flox declarations mean.** Every field flox
  documents today keeps working exactly as it does.

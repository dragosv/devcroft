# Change: add-mise-daemons

## Why

The owner asked on 2026-11-16 for mise including its new experimental
services. This change carries that half. It was split from `add-mise-provider`
on 2026-11-17, after an adversarial review found that every hard problem in
the service design was independent of the provider and that four of them
needed decisions rather than measurements:

- `process-compose` has no TCP readiness probe, and the first draft forbade
  both ways of approximating one.
- Honouring a declared shutdown grace is not a matter of reconciling two
  constants: three are involved, and the guarantee that no service survives
  `down` currently rests on the keeper's own sweep at a fixed two seconds.
- The field names the draft refused (`ready_timeout`, `startup_timeout`) do
  not exist upstream, and upstream defaults the draft promised to preserve
  include some devcroft's supervisor cannot express.
- The PostgreSQL and Redis presets install their servers through plugin
  backends, which `add-mise-provider` refuses.

Depends on `add-mise-provider`. No task here starts before that change's
Group 0 passes.

## What Changes

- Read `[daemons]` from the selected mise manifest as data, with devcroft's own
  parser, when the manifest opts in with `[settings] experimental = true`. No
  mise or pitchfork process is involved; mise never sees the table.
- Run the admitted daemons through devcroft's keeper-owned `process-compose`,
  which must be a locked tool of the project.
- Carry a stated field set: `run`, `dir`, `env`, `depends`, a fixed `port`,
  `init`, an integer `retry`, `ready_port`, `ready_cmd`, `ready_http` and
  `stop_signal`. Refuse every other key that declares something, by daemon and
  dotted key.
- Add a typed TCP readiness probe, rendered as an `exec` probe that runs
  devcroft's own binary. No shell idiom, no HTTP stand-in.
- Honour a declared shutdown grace up to a stated cap, by lengthening the
  keeper's wait before its sweep and `down`'s wait before killing the keeper.
  This changes behavior for **every** provider that declares a grace, devenv
  included, where it is clamped to one second today.
- Refuse presets (`postgres = "18"` and the table forms), task daemons,
  container images, references to other projects and providers, proxies,
  automatic ports and the per-check timeouts, each by name with its reason.
- Extend the cross-provider check to mise `[daemons]` under the condition it
  already has: only when the selected provider has no service concept.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `services`: mise daemon import, TCP readiness, declared shutdown grace.

Provider-scoped **ADDED** requirements, since the main spec tree is empty.
They extend `fix-lossy-service-translation` and `add-service-readiness`
without replacing their contracts. They supersede one requirement of
`add-mise-provider` (`config`: "mise daemons are refused until supported"),
which task 1.1 removes from that change's delta if it is still unarchived, or
marks REMOVED here if it has been synced.

## Impact

Expected implementation touches `src/provider/{mod,mise}.rs`,
`src/services/mod.rs`, `src/keeper/{mod,connection}.rs`,
`src/lifecycle/{up,terminate,ports,state}.rs` and `src/bin/devcroft.rs` (a
hidden `__probe-tcp` subcommand, beside `__keeper` and `__provision`, which
does not add to the command surface).

`down` can take longer than it does today: up to the declared grace when a
service ignores its stop signal. That is the feature, and it is bounded by the
cap.

Preset support is **not** in this change. A project that wants PostgreSQL
under mise declares the server as a tool from an admitted backend and a
`run` daemon for it. See design D8 for what would reopen presets.

See [design.md](design.md).

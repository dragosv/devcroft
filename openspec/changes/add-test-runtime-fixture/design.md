# Design — Test Runtime Fixture

## Context

The suite's provider dependence was measured, not estimated: **32 of 42 test
files** invoke `flox`/`nix`/`devbox` directly, and **7** of those are testing
a provider. The full suite is **159s warm on a Nix-capable host**, and on a
host whose Nix daemon is unreachable roughly 80 tests self-skip — which
`CLAUDE.md` already names as this project's characteristic failure mode ("a
green `cargo test` is not the same as a run that tested anything").

Everything below was checked against the code rather than taken from the
proposal's framing, and two checks changed the shape of the design.

**Every claim here about `up`'s behaviour is read from the source at the
line cited. Nothing in this document has been run yet** — Open Question 1
is the spike that decides whether the cheap row can exist at all, and no
task after it assumes the answer.

## Goals / Non-Goals

**Goals:**
- Let a neutral-surface test be written once and run against any provider row.
- Keep the default local row realistic (a real Nix flake), so `cargo test`
  during development still exercises a real closure, real loader, real store.
- Give CI a row that needs no Nix daemon, *if* one can exist without lying.
- Make an unavailable row visible per row, never silently substituted.

**Non-Goals:**
- Not a fourth provider, not manifest-selectable — see proposal Non-Goals.
- Not a replacement for the 7 provider-contract files.
- Not a claim that any synthetic row demonstrates closure-tier behaviour.
- **Not a redesign of `Provider`.** The trait grows only if the seam needs
  it; see D2.

## Decisions

## D1 — The seam is "all of `up` minus provider selection", not "pass a Resolution"

**Decision.** Extract `up_with_provider(...)` (internal, `test-support`-gated)
containing today's `up` body from provider resolution onward. Public `up()`
becomes: validate `env.provider`, build `ProviderKind`, call
`up_with_provider`. The injected path and the public path are then the same
code, not two paths that agree by discipline.

**Rationale.** The tempting smaller seam — hand `up` a ready-made
`Resolution` — leaves half the real path uncovered, because **`up` reaches
the provider through four independent entry points, not one**:

| what | where | why it matters |
|---|---|---|
| `ProviderKind::from_name` | `up.rs:166` | rejects unknown providers |
| `Provider::resolve` | `up.rs:167` | the only trait method today |
| `provider::manifest_fingerprint` | `up.rs:175` | free fn; staleness, recorded in `Meta` |
| `ProviderKind::static_name` | `up.rs:373` | becomes `Origin::Provider` in the compiled policy |
| `provider::services_declared_by_flox` | `up.rs:784` | free fn, inside `prepare_services` |

A `Resolution`-only seam covers row 2 and leaves fingerprint, rule origins
and service declaration on the real-provider path — so a fixture row would
silently test a *different* composition than production. This project already
treats that class of bug as the one to design out rather than watch for:
`resolved_grants` and `to_capability_set` share one resolver specifically so
Landlock's grants and the mount view cannot diverge.

**Alternative considered and rejected: `up_with_resolution(...)`.** Cheaper,
and genuinely useful for a narrower band of tests (hooks, `services::
render_config`, keeper-direct, `spawn_keeper`) that never need `up` at all —
those should use it, and that part needs no refactor. Rejected as *the* seam
because of the four-entry-point table above.

**The hard constraint on either shape:** the seam must not skip
deny-overlap validation, the mount probe, listener-before-restriction, or
hook ordering. A future step added to `up` and forgotten in the injected
path is exactly the drift this design is trying to prevent, which is why
D1 puts them in one function rather than two.

## D2 — Grow the fixture trait, not `Provider`

**Decision.** `Provider` keeps its single `resolve` method. The new
`ProviderFixture` trait is test-side and carries what a *row* needs:
`setup` (returns `None` = row unavailable, skip), `mutate_to_drift` (for the
staleness test), `name`, and `capabilities`.

**Rationale.** `setup` and `mutate_to_drift` are not provider behaviour —
they are "how do I get a project of this kind into a known state", which
differs radically per row (`flox init && flox install …`, `nix flake lock`,
`devbox install`, instant for the synthetic row) and has no meaning in the
product. Putting them on `Provider` would add product surface that exists
only for tests. Whether `fingerprint`/`static_name` move *onto* `Provider`
is a smaller, separable question (D1's table shows they are free functions
today); the fixture can call them either way.

**Capability gating lives in the fixture, never in the test.** A neutral
test must not read `if fx.name() == "nix"`. It asks
`fx.capabilities().services` and skips if absent — otherwise per-provider
conditionals re-enter through the back door and the matrix stops being a
matrix.

## D3 — Default row is Nix flakes; the cheap row must be asked for

**Decision.** `DEVCROFT_TEST_PROVIDER` unset ⇒ the **Nix flake** row.
`=test` ⇒ the synthetic row. `=flox|nix|devbox` ⇒ that row. `=all` ⇒ every
available row, with a per-row summary.

**Rationale.** Nix flakes is the most neutral *real* substrate: no flox
derived-hook-free semantics, no devbox lock preservation, and
`print-dev-env --json` returns the environment as data with the `shellHook`
inert. It gives a real closure, so the default local run still covers
`/nix/store` grants, a shell resolved out of the closure, a real dynamic
loader, and canonicalization.

**No automatic fallback, and this is the load-bearing half of the decision.**
If the default row's setup fails, the run **fails loudly** with the
`DEVCROFT_TEST_PROVIDER=test` remedy — it does not quietly downgrade. A
fallback would rebuild the exact failure mode this change exists to remove:
green, and nothing ran.

## D4 — The synthetic row is blocked by two of `up`'s own invariants, and they refuse rather than degrade

**This is the finding that most changes the proposal as originally argued**,
and it is why Open Question 1 gates the whole cheap-row half.

- **`shell::resolve` (up.rs:226).** Both halves are store-bound:
  `resolve_on_path` accepts a `PATH` hit **only if it canonicalizes under
  `/nix/store`** (`STORE_PREFIX`), and `resolve_in_closure` walks
  `nix-store --query --requisites`. The result is `ok_or_else`'d into a hard
  error. So a fixture whose shell is a static binary outside the store does
  not weaken an invariant — **`up` fails outright**. The guard is deliberate
  and measured: without it this function picked `/usr/bin/dash` for
  `samples/flox-services-sample` and every service died.
- **`prepare_services` (up.rs:836).** Fails at layer `provider` unless
  `process-compose` resolves *from the provider's own environment*
  (`services::resolve_in_env`, which deliberately ignores the host `PATH`).

**Three ways out, and the choice is deferred to the spike rather than
guessed:**

1. **Generalize the guard from `/nix/store` to "inside a path the provider
   declared".** Arguably closer to the invariant's actual intent — the rule
   is "the shell comes from the environment, not the host", and the store
   prefix is a proxy for that. Attractive, and *not* to be done casually:
   it edits the function `CLAUDE.md` calls "the whole correctness", so it
   needs its own measurement showing a host shell is still refused.
2. **Ship the synthetic row a real store-shaped closure** — i.e. it stops
   being no-Nix, which defeats its purpose.
3. **Scope the synthetic row out of `up` entirely**, restricting it to the
   band that never calls `up` (keeper-direct, `services::render_config`,
   hooks, policy compilation). Honest, much cheaper, and covers less.

**Nothing downstream assumes which one wins.** The matrix over the three
real providers is valuable on its own and is not blocked by any of this.

## D5 — Two axes, not one

The proposal's "neutral surface" is neutral across *providers*. It is not
neutral across *platforms*, and `add-macos-unix-socket-scoping` measured why:
on macOS, pty sessions are refused outright (`openpty` needs the pty slave;
only the master is granted), per-port bind scoping does not exist, host
binaries execute at ungranted paths, and symlinked path spellings are
refused. Several tests in the neutral band are already `cfg`-gated or
skipped there for reasons that have nothing to do with providers.

**Decision.** `ProviderCapabilities` is the provider axis; platform gating
stays where it already is (`cfg!`/documented skips pointing at
`docs/known-gaps.md`). They are not merged, because a platform gap is a
published product limitation with an owner, while a missing provider
capability is a property of the row. Conflating them would let a real macOS
gap hide inside "this row doesn't support that".

## D6 — Feature-gated, `#[doc(hidden)]`, never manifest-reachable

`test-support` is a non-default Cargo feature exposing a `#[doc(hidden)] pub
mod test_support`. `tests/` compiles the library as a dependency *without*
`cfg(test)`, so `cfg(test)` cannot carry this — but the feature must stay
off by default so `cargo build` and the published binary never contain it.
`ProviderKind` gains no variant and `config::parse` gains no accepted value:
the seam is an internal API, not a schema extension. That is what keeps "no
non-reproducible mode" true rather than nearly true.

## Risks / Trade-offs

- **[Risk] The fixture invents a `Resolution` no real provider emits.** Six
  fields (`env`, `unset`, `read_only_grants`, `activation_script`,
  `services`, `ran_activation_hook`); a hand-written combination can be
  internally consistent and unreachable in practice, so tests pass against a
  contract that does not exist. → **Mitigation**: the matrix inverts this.
  The same neutral tests run against real rows, so a fixture that drifts
  from reality shows up as a red row rather than as invisible over-permission.
  This only works if the real rows are actually run in CI, which is why they
  are required jobs, not optional ones.
- **[Risk] Wall-time multiplies by N under `=all`.** Today the neutral suite
  runs once; with four rows it runs four times, and each real row pays its
  own `flox install`/`devbox install`. → **Mitigation**: `=all` is a CI
  mode, not the local default; local is one row. Measure before adopting
  `=all` anywhere on the critical path.
- **[Risk] The synthetic row becomes the only one that ever runs**, with the
  real rows quietly skipping — a matrix that is green because it is empty.
  → **Mitigation**: per-row skip reporting is a spec requirement, not a nicety,
  and a real row that is *available but failing* must fail the run rather
  than skip.
- **[Trade-off] The default gets slower to set up, not faster.** Requiring a
  real Nix flake locally and failing loudly without it is worse ergonomics
  than a silent fallback, and deliberately so: the alternative is a
  developer believing they ran the realistic suite when they ran the cheap one.

## Migration Plan

Test files move row by row, not in one sweep: each file that moves must
still pass on the *real* provider it used to hardcode before it is allowed
to run on any other row. A file that only ever passes on the synthetic row
is a regression in coverage wearing the shape of a migration.

## Open Questions

1. **Can the synthetic row exist at all?** (D4.) The spike: build a fixture
   whose `PATH` points outside `/nix/store` and call the real `up`. Confirm
   it fails at `shell::resolve` as read, then decide between generalizing
   the guard, dropping the row, or scoping it below `up`. **Everything about
   the cheap row waits on this; the real-provider matrix does not.**
2. **Does `=all` earn its cost?** Needs the measured delta — wall-time for
   four rows against one, and how many currently-skipped tests become
   runnable — before it goes anywhere near a required job.
3. **Do `fingerprint` and `static_name` belong on the `Provider` trait?**
   Free functions today. Moving them removes the dispatch duplication D1's
   table describes, but widens a product trait for a test-driven reason.
   Decidable once the seam exists, not before.

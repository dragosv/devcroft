## Why

**32 of 42 test files build a real flox/nix/devbox environment; only 7 of them
are testing a provider.** The other 25 construct a closure — network, Nix
daemon, `flox install` — purely to get past `up`, and then assert something
that has nothing to do with providers: lock lifecycle, `down`, signal
forwarding, SSH channels, concurrency, hooks, exec, mount view, policy
rendering. That tax is paid three ways: wall-time (159s warm on a
Nix-capable host), fragility, and — worst — silence, because every one of
those files self-skips on a host whose Nix daemon is unreachable, which is
~80 tests reading exactly like passes.

This change does not make devcroft's testing *less* real. It makes the
realism land where it proves something, and stops the neutral surface from
being hostage to a provider it is not testing.

## What Changes

- **NEW** `test-runtime-fixture`: a fixture contract with one row per
  provider, selected by `DEVCROFT_TEST_PROVIDER`. A test in the neutral
  surface is written once, against `&dyn ProviderFixture`, and runs on
  whichever row is selected. **The default row is a real Nix flake**, not
  the synthetic one — local `cargo test` should stay realistic, and the
  cheap row must be asked for.
- **NEW** `provider-injection-seam`: the internal, feature-gated API a
  fixture injects through. Not a fourth `env.provider`, not reachable from
  a manifest, and — the load-bearing part — **not a shortcut around any
  enforcement step `up` performs**.
- **NEW**, and gating everything above: a spike that answers whether the
  no-Nix row can exist at all. Two of `up`'s own invariants currently
  refuse it outright, not weakly — see Impact. If the answer is no, the
  change still delivers the matrix over the three real providers, and says
  so rather than shipping a row that silently degrades.
- Test files move from "hardcoded flox" to the fixture. The 7
  provider-contract files stay exactly as they are, and keep being the
  authority on their own provider's behaviour.
- **No automatic fallback.** If a selected row's setup fails, the run fails
  or skips *visibly, per row* — never silently substitutes a cheaper row.
  A green run must not be able to mean "nothing ran".

## Capabilities

### New Capabilities

- `test-runtime-fixture`: the row contract — what a fixture must provide,
  how a row is selected, what "neutral surface" means and where its
  boundary is, and how an unavailable row is reported so it cannot read as
  a pass.
- `provider-injection-seam`: the internal seam — what it must expose, what
  it must refuse to expose, and the congruence requirement that keeps the
  injected path and the public `up` path from drifting apart.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced capability specs yet, so there
  is no main spec to delta against. The requirements this change *pressures*
  live in unarchived changes and are named under Impact instead, deliberately
  rather than by omission: one of them may have to change, and that is a
  decision this change must make explicitly, not absorb.)

## Impact

- **Affected code**: `src/lifecycle/up.rs` (the seam — extract orchestration
  from provider selection), `src/provider/mod.rs` (the entry surface is four
  functions, not one — see design.md), `Cargo.toml` (a non-default
  `test-support` feature), and ~25 test files.
- **Two invariants stand directly in the way of the no-Nix row, and neither
  degrades gracefully — they refuse:**
  - `crate::shell::resolve` (called at `up.rs:226`, its result `ok_or_else`'d
    into a hard error) only ever returns a shell that canonicalizes inside
    `/nix/store`; `resolve_in_closure`'s fallback queries store requisites.
    A fixture whose shell is a static binary outside the store cannot bring
    up a sandbox **at all**. This is not "the invariant loses coverage" — it
    is `up` failing. Changing that guard means touching the function
    `CLAUDE.md` calls "the whole correctness", so it is a decision for
    design.md, not a detail.
  - `prepare_services` fails at layer `provider` when `process-compose` is
    absent from the resolved environment (`up.rs:836`). A no-Nix row that
    wants to exercise services must ship a real `process-compose`, or that
    row cannot cover services.
- **A second axis nobody has accounted for: the platform.** `add-macos-unix-
  socket-scoping` measured five macOS gaps that make parts of the *neutral*
  surface non-runnable there regardless of provider — pty sessions are
  refused outright, per-port bind scoping does not exist, host binaries
  execute at ungranted paths. "Neutral across providers" is not "neutral
  across platforms", and a matrix that models only the provider axis will
  rediscover this one test at a time.
- **Not a user-visible change.** `ProviderKind` stays `Flox | Nix | Devbox`,
  `config::parse` keeps rejecting anything else, and no manifest can select
  a fixture. Nothing here alters what a user of the published binary gets.
- **Sequencing**: after `add-mount-isolation` and `add-macos-unix-socket-
  scoping` (both complete), before `add-agent-workload`. Ahead of 0.3
  because fleet and agent work multiply lifecycle × namespace × services ×
  policy combinations, and migrating the suite after that is strictly more
  work. **Not** a blocker for cutting 0.1.0: that release is gated on the
  boundary being what the documents say, which this change does not touch.

## Non-Goals

- **Not a fourth provider.** No `env.provider = "test"`, no
  `ProviderKind::Test`, nothing selectable from `devcroft.toml`. The
  "declarative, lockfile-backed, reproducible" contract every provider must
  meet is not being given an exception; it is being kept out of reach.
- **Not a replacement for provider-contract tests.** Environment capture,
  lockfile preservation, the flox hook-free derive, `print-dev-env --json`,
  `shellenv --pure` — these are provider behaviour and stay real, mandatory,
  and per-provider.
- **Not a claim that the synthetic row proves closure-tier behaviour.** It
  cannot, by construction: a static binary has no dynamic loader, so it
  never exercises the `/lib` → `ld-linux` → merged-`/usr` path that
  `fleet::mount::setup_merged_usr_compat` exists for. Whatever that row
  covers, "a real toolchain runs inside the mount view" is not it.

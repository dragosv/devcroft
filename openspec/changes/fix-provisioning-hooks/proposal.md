# Change: fix-provisioning-hooks

Status: proposed. Affects `add-mvp-core` (flox) and `add-nix-provider`
(nix) — both implemented and shipped. Found while measuring devbox for
`add-devbox-provider`, which asked this question of a provider for the
first time.

## Why

**Both shipped providers execute arbitrary project-supplied shell during
`up`, host-side, before any restriction exists.** Measured, not
inferred, against the exact commands in `src/provider/`:

- `flox activate -- env -0` (`flox.rs`) runs the flox manifest's
  `[hook].on-activate`.
- `nix develop --no-update-lock-file --command sh -c 'env -0'`
  (`nix.rs`) runs the devShell's `shellHook`.

In both cases a hook appending to a sentinel file ran during provider
resolution, with the invoking user's full network access and full
filesystem write access — no Landlock, no sandbox, nothing.

This contradicts an invariant CLAUDE.md states as non-negotiable, and
specifically voids the reason the invariant gives for trusting the
provisioning phase at all:

> Provider provisioning (package materialization, environment capture)
> runs host-side at `up`, *before* restrictions, using the host's own
> network — **trusted because it executes pinned tooling from a
> lockfile, not project code.**

An `on-activate` block is project code. So is a `shellHook`. The
justification does not hold, and the consequence is concrete: `devcroft
up` on a repository you have not read is arbitrary code execution as
your user. A tool whose entire pitch is running project code under
confinement should not execute that code unconfined on the way to
setting the confinement up.

It is worth being precise about how much is new. A developer who runs
`flox activate` or `nix develop` by hand already accepts this, and
devcroft is not making that workflow worse. What is new is the
*expectation* devcroft creates: users are told sandboxes exist so
project code cannot touch the host, and nothing tells them the setup
step is exempt.

## What Changes

- **nix is fixed properly.** Resolution switches to
  `nix print-dev-env --json`, which emits the build environment as
  structured data and never executes the hook — measured. The
  `shellHook` arrives as an inert JSON string. devcroft reads the
  variables directly with the JSON parser it already links; no shell
  evaluates anything.
- **flox cannot be fixed the same way, and this change says so rather
  than pretending otherwise.** Measured: no `flox activate` mode
  suppresses `on-activate` — not `--mode run`, not `--mode dev`, not
  `--no-start-services`. flox's own docs note that `-- <cmd>` "does not
  run any profile scripts", which is true and does not cover `[hook]`.
- **`up` therefore detects and reports it.** Where a provider's
  environment definition contains an activation hook devcroft cannot
  skip, `up` prints exactly one warning naming the provider, the
  construct, and what it means — the same "degraded capabilities are
  surfaced, never silent" contract already used for unenforceable
  policy aspects. It does not fail: refusing every flox project that
  uses `on-activate` would reject an idiomatic, widely-used feature,
  and the user's own `flox activate` does the same thing.
- **The invariant text is corrected.** CLAUDE.md's two-phase paragraph
  currently asserts something false. It states the real contract
  instead: provisioning runs pinned tooling *and* whatever activation
  hook the provider's environment defines, which devcroft avoids where
  the provider offers a way and reports where it does not.
- **The published limitations gain this entry**, because it is exactly
  the kind of thing the README's own Limitations section exists for.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `env-provider`: adds a requirement that provisioning not execute
  project code where the provider permits capturing without it, and
  that it be reported where it does not. Modifies the nix resolution
  requirement, whose capture mechanism changes.
- `cli`: `up`'s warning surface gains the activation-hook notice.

## Impact

- **Affected code**: `src/provider/nix.rs` (capture mechanism),
  `src/provider/flox.rs` (hook detection), `src/provider/mod.rs`
  (carrying the notice out of resolution), `src/lifecycle/up.rs` and
  `src/bin/devcroft.rs` (printing it once).
- **Affected docs**: CLAUDE.md's two-phase invariant, README
  Limitations, `docs/decisions.md` §1 — the six-criterion test should
  say that criterion 4 is about capturing activation *without executing
  project code*, which is what it always meant and never said.
- **No behavior change for projects without activation hooks**, which
  is the regression test: same captured environment, same compiled
  policy, byte for byte.
- **Interacts with `add-devbox-provider`**: that change already specs
  this rule for devbox (its `env-provider` delta, "Provisioning never
  executes project code") and picks `shellenv --pure` to satisfy it. Its
  spec becomes the general rule here rather than a devbox-specific one.
  This change should land first.

## Success Criteria

- A nix project whose devShell defines a `shellHook` that writes a file
  comes up with that file **not** created, and with an environment
  otherwise identical to what the previous mechanism captured.
- A flox project whose manifest defines `[hook].on-activate` comes up,
  and `up` prints exactly one warning naming the hook — not zero, and
  not one per session.
- A project with no activation hook produces a byte-identical captured
  environment and compiled policy before and after this change.
- `nix print-dev-env --json`'s `exported` variables reproduce the
  current capture: measured at 74 keys against 74, differing only in
  shell bookkeeping (`PWD`, `SHLVL`, `_`, `OLDPWD`) that has no business
  in an env diff.
- CLAUDE.md no longer claims provisioning executes no project code.

## Open Questions

- Whether the flox warning should be suppressible once acknowledged.
  Leaning no for now: a warning that can be turned off for a property
  that is still true is how this became invisible in the first place.
- Whether `bashFunctions` from `print-dev-env --json` should be
  captured too. The current mechanism cannot see them at all, so
  dropping them preserves today's behavior; adopting them would be a
  separate, additive decision.
- Whether flox exposes any non-CLI path to a hook-free capture (reading
  its own generated activation script, for instance). Not investigated,
  because it would mean depending on a flox internal — the same
  undocumented-artifact trap `add-flox-services` decision 1 already
  rejected once.

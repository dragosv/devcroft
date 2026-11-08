# Change: add-devbox-provider

Status: proposed (post-MVP). Depends on: `add-nix-provider` (complete).
This is provider #2 on `openspec/config.yaml`'s roadmap, and the third
closure-tier provider overall — the second one that exists purely to
confirm the `Provider` trait generalizes, with no new guarantee concepts.

## Why

devbox is the largest population of declarative Nix environments that
`add-nix-provider` does *not* serve. A devbox project has no `flake.nix`
to point `nix develop` at: `devbox.json` names packages against nixpkgs,
`devbox.lock` pins them per-system, and devbox synthesizes the
environment itself. Those users get devcroft's "not yet supported"
rejection today (`provider::validate`'s `NOT_YET_SUPPORTED`), despite
being on exactly the substrate devcroft's closure tier is built for —
the same `/nix/store`, the same transitive hashing down through libc.

It is also the cheapest qualified provider remaining. `add-nix-provider`
already paid the generalization cost: provider dispatch, per-provider
fingerprinting, and grant attribution are all keyed off the provider name
in one place (`provider::mod`), and the `Resolution` contract (env diff,
unsets, read-only grants, service support) did not change when nix
landed. devbox reuses the store-grant derivation and the fixed-baseline
env-diff capture unchanged; what is genuinely new is one activation
command and one pair of hashed files.

Two things make this worth proposing now rather than later:

- **It is the honest test of the "third provider is free" claim.** nix
  was picked as the second provider partly because it is the substrate
  flox sits on, so a shared assumption could have hidden inside both.
  devbox activates differently enough (its own resolver, its own
  lockfile format, no flake) to catch an assumption that survived nix.
- **It surfaces a claim already made on devbox's behalf.** The
  `add-flox-services` design asserts one config generator serves "flox,
  devbox (which is process-compose-based too)". That assertion has never
  been checked. This change is where it gets checked — and, as the
  Impact section explains, the first cut deliberately does *not* depend
  on it holding.

## What Changes

- **`env-provider` gains provider `devbox`**, closure tier — the same
  tier as flox and nix, so no user-visible tier machinery is introduced
  and `docs/decisions.md` §1's artifact-tier host-grant rule does not
  apply. Nothing new appears in `policy --render` beyond a store closure
  attributed `provider:devbox`.
- **Resolution captures devbox's activated environment once, host-side,
  at `up`**, diffed against the same fixed canonical baseline `flox.rs`
  and `nix.rs` share, so the captured diff is independent of the
  operator's shell. The concrete capture command is a decision for
  design.md and is gated on live measurement (see Open Questions).
- **Store grants** come from the resolved closure's `/nix/store` paths,
  derived by the same mechanism the other two closure providers use,
  annotated `provider:devbox`.
- **Preconditions, all checked at `up`, layer `provider`, exit code 3:**
  - `devbox` on PATH (hint: `devcroft doctor`).
  - `nix` present — devbox is a frontend over Nix and cannot materialize
    anything without it. Reported as devbox's precondition, not as a
    demand that the project declare `provider = "nix"`.
  - `devbox.json` present; a missing one is a missing environment (hint
    `devbox init`), not a missing feature.
  - Every declared package already has a key in `devbox.lock`, **and**
    capture leaves `devbox.lock` byte-identical; otherwise `up` fails
    rather than resolving versions against whatever nixpkgs currently
    points at. Deliberately not "resolved for the running system":
    measured against devbox 0.18.0 with a cold store, a lock entry
    resolved only for another platform still resolves correctly here,
    from its pinned commit reference, without touching the file. The
    byte comparison is what actually enforces the rule — the per-package
    check cannot see devbox's own base nixpkgs entry, which is where the
    shipped implementation was found to still be resolving live. See
    design.md decisions 1b and 1c.
- **Staleness**: fingerprint of `devbox.json` + `devbox.lock`, the same
  contract flox has over `manifest.toml` + lockfile and nix has over
  `flake.nix` + `flake.lock`.
- **Services are `ServiceSupport::Unsupported` in this change**, matching
  what nix does today and deliberately *not* what
  `add-flox-services` predicts. See Impact — this is a scope decision,
  not an oversight, and it is the one place this change declines to
  inherit an existing claim.
- **`doctor`** learns a devbox check, reported only for projects that
  declare `provider = "devbox"` — the per-provider scoping `doctor`
  already applies to flox and nix.
- **`init`** detects an existing `devbox.json` and offers `devbox`.

## Capabilities

### New Capabilities

None. devbox is a third implementation of the existing `env-provider`
capability, not a new capability — the same reasoning
`add-nix-provider` gave for nix.

### Modified Capabilities

- `env-provider`: adds a "devbox provider resolution" requirement
  (fixed-baseline activation capture, store grants, lockfile
  precondition, nix precondition, staleness), and narrows "Only
  declarative providers" — `devbox` moves from a "not yet supported"
  rejection to an accepted value.
- `config`: `env.provider`'s accepted value set widens to include
  `devbox`.
- `cli`: `doctor` gains a devbox check scoped to projects declaring it;
  `init` detects `devbox.json`.

## Impact

- **Affected specs**: `env-provider` (new requirement + modified
  rejection requirement), `config` (provider value set), `cli`
  (`doctor`, `init`).
- **Affected code**: `src/provider/devbox.rs` (new), `validate.rs`
  (one name moves out of `NOT_YET_SUPPORTED` into `SUPPORTED`),
  `mod.rs` (`ProviderKind::Devbox`, dispatch, `static_name`,
  `manifest_fingerprint`), `doctor`/`init` in `src/bin/devcroft.rs`.
  No changes to lifecycle, exec, ssh, or policy compilation — as with
  nix, that absence is the result this change exists to demonstrate.
- **Deliberately out of scope: devbox services.** `add-flox-services`'
  design says its generator serves devbox because devbox is also
  process-compose-based. That is true of devbox's *supervisor* and does
  not settle the question this change would have to answer, which is
  where the *declarations* come from. flox qualified because it has a
  documented `[services]` schema in its own manifest; devbox's services
  come from plugins that ship their own process-compose configs, plus an
  optional project-root `process-compose.yaml`. Consuming those means
  reading a process-compose config directly — which is the shape
  `add-flox-services` decision 1 explicitly rejected for flox, on the
  grounds that an internal generated artifact is not a contract.
  Resolving that is a separate change with its own decision to make;
  bundling it here would let this provider land on an unexamined claim.
  Until then `provider = "devbox"` with a manifest declaring services
  fails the same way `nix` does, which the `services` spec already
  requires be distinguishable from "supports services, none declared".
- **Unblocks**: nothing on the critical path. `devenv` (also closure
  tier, also nix-based, also service-bearing) becomes cheaper, and the
  devbox services question above is the same question devenv's own
  deferral names.

## Success Criteria

- A project with `devbox.json` + `devbox.lock` and
  `env.provider = "devbox"` comes up; `devcroft exec` sees the devbox
  environment's toolchain; every tool runs under
  `network.default = "deny"`, because materialization happened host-side
  at `up`.
- The captured env diff is byte-identical regardless of the invoking
  shell's own PATH and environment — the same guarantee flox and nix
  have, verified the same way (`tests/flox_env_capture_is_deterministic.rs`
  has the shape).
- A full build inside the sandbox needs no host library grants: the
  compiled policy grants the project root, `/tmp`, and the devbox
  closure's store root, and `/usr/bin/gcc` is denied — the same
  measurement `own-policy-baseline` recorded for flox and nix, which is
  what makes "closure tier" a claim about this provider rather than
  about Nix in the abstract.
- Removing `devbox.lock` from a project that declares packages fails
  `up` at layer `provider`, hinting `devbox install` (devbox has no lock
  subcommand); removing `devbox.json` fails with a hint to `devbox
  init`. A project declaring no packages needs a lockfile too — devbox's
  stdenv comes from a base nixpkgs entry that is unpinned until one
  exists — so it fails the same way until `devbox install` has run.
- `policy --render` shows the store grants with origin `provider:devbox`;
  provider resolution adds no write grants.
- Editing `devbox.json` or `devbox.lock` flips `status` to stale and
  `up` prints the `--recreate` notice.
- `doctor` in a devbox project reports devbox and stays silent about
  flox — the per-provider scoping already in place.
- **`src/provider/mod.rs`'s dispatch is the only shared file that
  changes shape.** If implementing devbox forces a change to
  `Resolution`, to `policy::compile`, or to `lifecycle::up`'s provider
  handling, the "the trait generalizes" claim is weaker than stated and
  that finding is recorded rather than absorbed.

## Open Questions

- **Which capture command.** `devbox shellenv` prints shell `export`
  statements to be `eval`'d, not an environment dump — closer to nix's
  `print-dev-env` than to a clean `env -0`. `devbox run -- sh -c 'env -0
  > <tmp>'` would reuse `nix.rs`'s existing trick exactly. Which is less
  fragile across devbox versions has to be **measured, not chosen from
  the docs**; see the first task group.
- **Whether `devbox shellenv --init-hook` / `shell.init_hook` runs during
  capture, and whether it should.** An init hook is project code, and
  devcroft's two-phase rule says project code never runs in the trusted
  provisioning phase. If capture cannot avoid running it, that is a
  qualification problem for devbox, not a detail — and it would be the
  first criterion-4 failure devcroft has found in a nix-based provider.
- **Whether devbox's global/default packages leak into the capture.**
  devbox has a machine-global package set; if activation includes it,
  the environment is not a pure function of the committed files and
  criterion 2 is compromised. Needs checking against a real install.
- **Nothing in this proposal has been verified against a running
  devbox.** It is not installed in this repo's devcontainer, and every
  statement above about devbox's CLI, file names, and behavior comes
  from its documentation. That is a weaker footing than
  `add-nix-provider` had, and the task list is ordered so the
  measurements happen before any code depends on them.

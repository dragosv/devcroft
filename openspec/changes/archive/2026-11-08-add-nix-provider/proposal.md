# Change: add-nix-provider

Status: proposed (post-MVP). Depends on: add-mvp-core complete. This is
the provider `add-mise-provider` names as its own prerequisite — the
second closure-tier provider that proves the `Provider` trait generalizes
beyond flox, without yet introducing mise's new concepts (guarantee
tiers as user-visible state, devcroft-owned store).

## Why

Nix flakes are the largest existing population of declarative dev
environments: every repo with a `flake.nix` exposing a `devShell` is
already a complete, lockfile-pinned closure-tier environment — devcroft's
own six-criterion provider test (docs/decisions.md §1) passed by the
substrate flox itself builds on. Today devcroft tells those users
"not yet supported"; they are the cheapest qualified users to serve,
since supporting them requires no new guarantee concepts, no new store
model, and no manifest translation — only a second implementation of the
existing `Provider` trait.

## What Changes

- `env-provider` gains provider `nix` (accepting the aliases `flake` and
  `flakes` already reserved in `provider::validate`), closure tier — the
  same tier as flox, so no user-visible tier machinery is introduced.
- Resolution runs `nix develop` (or `nix print-dev-env`) for the
  project's flake once at `up`, host-side, before restriction — the same
  fixed-environment env-diff capture flox uses, against the same
  canonical baseline, so the captured diff is independent of the
  operator's shell.
- Preconditions enforced at `up` (layer `provider`, exit code 3):
  - `nix` binary on PATH with flakes enabled; hint `devcroft doctor`
    otherwise.
  - `flake.nix` present; a missing flake is a missing environment
    (hint `nix flake init`), not a missing feature.
  - `flake.lock` present and covering the flake's inputs; an unlocked
    flake fails `up` with hint `nix flake lock` rather than resolving
    inputs at `up` time to whatever the registry currently points at.
- Store paths: the resolved dev shell's `/nix/store` closure becomes
  read-only grants annotated `provider:nix`, same mechanism as flox's
  store grants.
- Staleness: fingerprint of `flake.nix` + `flake.lock`, same contract as
  flox's `manifest.toml` + lockfile fingerprint.
- `doctor` learns to check for a usable nix (binary present, flakes
  enabled, daemon reachable where applicable).
- Provider selection: `devcroft.toml`'s `env.provider = "nix"` becomes
  valid; `init` detects an existing `flake.nix` and offers `nix` where
  it currently only offers flox.

## Capabilities

### New Capabilities

None — nix is a second implementation of the existing `env-provider`
capability, not a new capability.

### Modified Capabilities

- `env-provider`: adds a "nix provider resolution" requirement (mirroring
  the flox one: fixed-environment activation capture, store grants,
  lockfile precondition, staleness) and narrows "Only declarative
  providers" — `nix`/`flake`/`flakes` move from "not yet supported"
  rejections to accepted values.
- `config`: `env.provider` value set widens to include `nix`.
- `cli`: `doctor` checks nix availability/flakes enablement; `init`
  detects `flake.nix`.

## Impact

- Affected specs: `env-provider` (new requirement + modified rejection
  requirement), `config` (provider enum), `cli` (`doctor`, `init`).
- Affected code: `src/provider/` (new `nix.rs`, `validate.rs` moves three
  names out of `NOT_YET_SUPPORTED`, `mod.rs` provider dispatch),
  `src/lifecycle/up.rs` (provider selection instead of hard-wired
  `FloxProvider`), `doctor` and `init` in `src/bin/devcroft.rs`.
- No changes to lifecycle semantics, exec, ssh, policy compilation — the
  provider contract (Resolution: env diff, unsets, read-only grants) is
  unchanged, which is the point: this change is the proof that the
  contract holds for a second provider.
- Unblocks: `add-mise-provider` (its stated dependency).

## Success Criteria

- A project with `flake.nix` + `flake.lock` and
  `env.provider = "nix"` comes up; `devcroft exec` sees the dev shell's
  toolchain; every tool runs under `network.default = "deny"` because
  materialization happened host-side at `up`.
- The captured env diff is byte-identical regardless of the invoking
  shell's PATH or environment (same guarantee flox has).
- Removing `flake.lock` fails `up` at layer `provider` with hint
  `nix flake lock`; removing `flake.nix` fails with hint
  `nix flake init`.
- `policy --render` shows the store closure grants with origin
  `provider:nix`; provider resolution cannot add write grants
  (the existing "Provider does not weaken the sandbox" requirement
  applies unchanged).
- Editing `flake.nix` or `flake.lock` flips `status` to stale and `up`
  prints the `--recreate` notice.

## Open Questions

- `nix develop --command` vs `nix print-dev-env`: print-dev-env emits
  the shell environment directly (cheaper, no pty), but its output is a
  bash script to be sourced, not a clean env dump; decide which capture
  mechanism is less fragile across nix versions.
- Whether to require the flake to expose `devShells.<system>.default`
  or also accept a manifest key naming a non-default shell
  (`env.shell = "ci"`). Leaning: default-only for the first cut, the
  key is additive later.
- Impure flakes (`--impure`, builtins reading the host environment)
  break the closure guarantee; decide whether to reject, warn, or
  ignore — rejecting is most consistent with "no non-reproducible mode",
  but detection may be limited to refusing the `--impure` flag rather
  than proving purity.
- Whether `nix` should also serve non-flake classic `shell.nix`
  projects. Leaning: no — no lockfile, fails criterion 2; the rejection
  message should say `nix flake init`, keeping the "not yet supported"
  vs "out of scope" distinction inside nix itself.

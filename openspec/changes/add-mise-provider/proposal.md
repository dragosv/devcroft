# Change: add-mise-provider

Status: proposed (post-MVP). Depends on: add-mvp-core complete, provider
trait proven on at least one additional closure-tier provider (nix flakes
or devbox), since mise introduces new concepts (tiers, devcroft-owned
store) on top of the trait.

## Why

mise is the lightest supported entry point: a single static binary, no
store daemon, no multi-GB `/nix`, and the largest user base of any
candidate provider. Supporting it widens devcroft's funnel considerably.

It cannot be supported naively without breaking devcroft's identity:
mise's native guarantee is artifact integrity (exact versions, URLs,
checksums, provenance via `mise.lock` + `locked` mode), not behavioral
reproducibility (Nix closure covers linking transitively; mise binaries
link against host libc/openssl). This change introduces mise under an
explicit, user-visible guarantee tier rather than pretending the two
guarantees are equal.

## What Changes

- `env-provider` gains provider `mise`, tier `artifact`.
- New concept: guarantee tiers. `closure` (flox, devbox, nix) vs
  `artifact` (mise). Tier is derived from the provider, shown in
  `status` and once at `up`, and documented in the README's honesty
  section.
- Preconditions enforced at `up` (all hard failures, layer `provider`):
  - `mise.lock` exists and covers the current platform; hint `mise lock`
    otherwise.
  - devcroft invokes mise with locked mode enforced (`MISE_LOCKED=1`);
    tools without pre-resolved URLs for the platform fail the up.
- Store management: devcroft owns a dedicated `MISE_DATA_DIR` under
  devcroft's data dir, shared across sandboxes, append-only by policy:
  devcroft only ever installs (at `up`, on the host, before sandbox
  application — initializeCommand semantics); it never upgrades, prunes,
  or self-updates there. Sandboxes receive a read-only grant on it,
  annotated `provider:mise`.
- Staleness: hash of `mise.toml` + `mise.lock`.
- Degradation surfacing: backends with partial lock support (e.g.
  checksum-only, no provenance) are listed once at `up` as a warning
  naming the affected tools.

## Impact

- Affected specs: env-provider (new requirements), policy (tier
  annotation), cli (`status` output), config (provider enum).
- No changes to lifecycle, exec, ssh.

## Success Criteria

- A project with `mise.toml` + complete `mise.lock` comes up with zero
  network access from inside the sandbox and `guarantee: artifact` in
  `status`.
- Removing `mise.lock` fails `up` with the `mise lock` hint.
- Two sandboxes share the devcroft-owned mise data dir read-only; a
  host-side `mise upgrade` in the user's own mise dir has no effect on
  running sandboxes.

## Open Questions

- Whether `guarantee` should also be declarable in the manifest as a
  ceiling (`guarantee = "closure"` making mise invalid for that project),
  so teams can enforce the stronger tier repo-wide.
- Interaction with aqua-backend metadata calls under locked mode
  (upstream issue): decide whether to tolerate, block via policy, or
  pin a minimum mise version where it is fixed.

## Why

`add-test-runtime-fixture` delivered a row contract and a matrix over flox,
nix and devbox — but **every row still needs a working Nix store**, so the
problem that motivated it is only half solved. The other half was always the
cheap row, and it was deferred there as group 5 because two of `up`'s own
invariants refused it.

One of those refusals is now gone: that change generalized `shell::resolve`
from a hardcoded `/nix/store` to "inside a path the provider declared", and
**a Nix-free row has since been measured to bring a sandbox up on macOS** —
`Ok(Started)`, with the resolved shell outside the store. The mechanism
works. What is left is a narrower and more tractable question than the one
group 5 was written against: *which binary goes in the row's directory?*

This matters beyond tidiness. Migrating the neutral surface to the fixture
is blocked on it: a migrated test uses the default row, which fails rather
than skips where no Nix store exists, so migrating today turns this
project's own devcontainer suite — ~80 tests that currently skip — from
green into red.

## What Changes

- **NEW** `nix-free-test-row`: a fixture row whose environment is a
  directory it owns and declares as its own grant, containing a real POSIX
  shell that came from neither the Nix store nor the ambient host `PATH`.
- **Per-platform by necessity, not by preference.** The Linux and macOS
  answers cannot be the same binary, and the change says so rather than
  describing one row with two materially different runtimes.
- Once the row exists, `add-test-runtime-fixture`'s group 4 unblocks: the
  neutral surface can migrate without breaking daemon-less hosts.

## Capabilities

### New Capabilities

- `nix-free-test-row`: what the row must provide, where its binaries may and
  may not come from, and what it is explicitly not evidence for.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs yet. This change extends
  `add-test-runtime-fixture`'s `test-runtime-fixture` capability with a new
  row rather than altering its requirements; that capability's "a row's
  realism is not weakened to make it pass" requirement already governs this
  row and is what rules out the obvious shortcut below.)

## Impact

- **Affected code**: `tests/common/mod.rs` (a fourth row plus its setup),
  and whatever mechanism supplies the row's binaries.
- **Four measurements already taken**, on macOS 15.7.4, which between them
  decide the shape:
  1. **A non-store row brings `up` to `Started`.** The generalized guard
     resolved a shell at `/private/tmp/…/bin/sh` and recorded it. The
     mechanism is proven; this is what makes the change tractable at all.
  2. **But the sandbox could not run anything**, because of (3).
  3. **Copied macOS platform binaries hang.** `cp /bin/sh` then running it
     never returns — and neither does a copied `/bin/echo`, so this is not
     specific to shells. It *hangs* rather than failing, which is the worst
     shape a fixture failure can take. Code signatures survive the copy
     intact (identical `CodeDirectory`), so signing is not the explanation.
     **This kills the "copy a shell from the host" option outright on
     macOS**, independently of whether that option was ever tasteful.
  4. **Freshly compiled binaries run fine** from an arbitrary directory. So
     building the row's shell from source is viable on macOS, where copying
     one is not.
- **Unblocks** `add-test-runtime-fixture` group 4 (migrating ~22 neutral
  files), which is deliberately stalled until a row exists that works
  without a Nix daemon.
- **Not a user-visible change.** No new `env.provider`, no new
  `ProviderKind` variant, nothing selectable from a manifest. This is test
  infrastructure.

## Non-Goals

- **Not evidence that a real toolchain works inside the mount view.** A
  static binary has no dynamic loader, so it never exercises the `/lib` →
  `ld-linux` → merged-`/usr` path `fleet::mount::setup_merged_usr_compat`
  exists for. Whatever this row proves, that is not it, and the real-provider
  rows stay required in CI for exactly that reason.
- **Not the default row.** `add-test-runtime-fixture` decided that local
  `cargo test` runs against a real Nix environment and the cheap row is
  asked for by name. Nothing here changes that; a fast row that quietly
  became the default would recreate the problem both changes exist to close.
- **Not a way to avoid the real providers.** The 7 provider-contract files
  stay hardcoded and real, and the per-provider CI jobs stay required.

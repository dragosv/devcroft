# Change: add-guix-provider

## Why

Every closure-tier provider devcroft ships (flox, nix, devbox, devenv) sits on
one substrate, Nix. `docs/decisions.md` §1 accepts that concentration risk
openly ("upstream churn in flakes, the daemon, or store semantics now hits
every provider devcroft ships simultaneously") and names it as a revisit
trigger. GNU Guix is the only other substrate with the same guarantee: a
content-addressed store, functional builds by a daemon, and substitutes. Its
pin is a channels file of exact commits (`guix describe -f channels`), replayed
by `guix time-machine`.

It is **not scheduled**. This proposal records what a Guix provider needs, so
that when demand or Nix churn makes it worth building, the work starts from a
measured gate rather than from scratch.

## What Changes

- Group 0 measures whether Guix passes the six criteria on a pinned Guix
  revision, on Linux. Guix does not run on macOS, so the provider is
  Linux-only and refused on macOS at layer `provider`, as `swift` is refused
  off macOS.
- If admitted: accept `env.provider = "guix"` as a **closure-tier** provider.
- Inputs: a `manifest.scm` and a `channels.scm`. Both are required, and the
  channels file must pin every channel to a commit. A channels file without
  commits, or a missing one, is refused. Without it, `time-machine` follows
  the user's own channel configuration and HEAD.
- Capture with `guix time-machine -C channels.scm -- shell -m manifest.scm
  --pure --search-paths`, which prints the environment's variable definitions
  instead of starting a shell. devcroft parses that output as data and refuses
  anything that is not a plain assignment.
- Resolution (evaluating `manifest.scm` and the channels' Scheme) runs host-side
  with the daemon. This is the category nix already has, and
  `sandbox-provisioning` P6 already states it. `policy --render` states it for
  Guix too.
- Grants: the store root `/gnu/store`, read-only, origin `provider:guix`. The
  daemon socket is never granted. Profiles are registered as GC roots under
  devcroft's state, so `guix gc` cannot remove a live sandbox's closure.
- The shell comes from the closure: `src/shell.rs` already accepts a hit inside
  any declared grant, not only `/nix/store`.

## Capabilities

### New Capabilities

None. This extends the existing provider contract.

### Modified Capabilities

- `config`: accepts `guix` and an optional `[env.guix]`.
- `env-provider`: pinned resolution, data-only capture, GC roots and the
  platform refusal.
- `policy`: the `/gnu/store` grant and the refusal of the daemon socket.

The main spec tree is empty, so these are provider-scoped **ADDED**
requirements.

## Impact

- `src/provider/{mod,validate,guix}.rs`; `src/provider/capture.rs`, whose store
  detection is the one nix-specific line in shared code (`DEFAULT_STORE_ROOT`)
  and must take the provider's store root; a `host_can_build_guix_closures()`
  probe beside `host_can_build_nix_closures()` for tests; `src/bin/devcroft.rs`
  (`init`, `doctor`, `status`); `tests/guix_provider_e2e.rs`;
  `samples/guix-sample`.
- `fleet::mount::construct_view` must expose `/gnu/store` and hide
  `/var/guix/daemon-socket`. This mirrors what it already does for
  `/nix/store` and the nix daemon socket.
- `docs/decisions.md` §1 gains an entry, "Built: guix" or a rejection naming the
  property that fails. The "nix monoculture" paragraph is updated either way.
- Developers and CI need a Linux host with `guix-daemon`. The first `up` of a
  given channels pin may build or fetch that Guix revision, which is slow.
  `doctor` states it.
- No services. Guix's service model (Shepherd) belongs to `guix home` and
  `guix system`, not to `guix shell`.

See [design.md](design.md) for what is documented, what must be measured, and
the stop criteria.

## Why

**Landlock does not mediate `connect()` to a pathname unix socket; `add-mount-isolation`
closed that on Linux with a per-sandbox mount namespace — a mechanism macOS has no
equivalent of.** Seatbelt has no user/mount namespace primitive, so `up_process` now
degrades gracefully there instead of failing every sandbox (the regression
`add-mount-isolation` shipped and this change's own prerequisite fix corrected): every
macOS sandbox is back to Seatbelt-only, and the AF_UNIX gap is exactly as open as it has
always been on that platform.

Closing it does not need a namespace, though — Seatbelt classifies unix-socket
`connect()` as network activity, not filesystem access, so the mechanism devcroft
already has (`network.default = "deny"`) may already reach it. That is a real,
different-shaped answer to the same gap, not a port of the Linux fix, and it is the
subject of this change.

## What Changes

- **NEW** `macos-unix-socket-mediation`: on macOS, `network.default = "deny"` denies
  `connect()` to any pathname unix socket the sandbox was not explicitly granted, the
  same guarantee `add-mount-isolation`'s mount view gives on Linux, delivered instead
  through Seatbelt's own network-outbound classification.
- **NEW**, and load-bearing before anything above is a claim rather than a proposal: a
  spike on real macOS hardware. **Done — and the premise that "this repository has no
  macOS host" was itself false.** Measured on macOS 15.7.4 (arm64) against a live
  `nix-daemon` socket: the mechanism holds, and it required no policy-compilation code
  because devcroft already compiled the rule that closes it. What the spike changed:
  the proxy-socket grant below is **not** implemented (macOS reaches its proxy over
  TCP, never over the unix socket), and a prerequisite nobody had noticed *was*
  implemented — the crate did not compile on macOS at all, so `add-mount-isolation`'s
  macOS degradation had never actually run.
- **MODIFIED** `filesystem-view` (the capability `add-mount-isolation` added): its own
  spec is Linux-specific in mechanism (a mount namespace); this change does not alter
  that spec, but the proxy-socket exception it documents (M3) gets a macOS-shaped
  sibling here — a scoped `UnixSocketCapability` grant rather than a bind mount, since
  Seatbelt has no view to bind into.
- **MODIFIED** `cli`: `tests/unix_socket_not_mediated.rs` currently asserts a
  Linux-specific failure shape (`ENOENT` — the path does not resolve, per the mount
  view). macOS's correct refusal is a different mechanism entirely (`EPERM` from
  Seatbelt's network deny, not "the path does not exist") and needs its own assertion,
  not a shared one that happens to pass on both platforms by coincidence.

## Capabilities

### New Capabilities

- `macos-unix-socket-mediation`: pathname unix-socket connect() denied by default on
  macOS via `network.default = "deny"`, with devcroft's own proxy socket admitted
  through an explicit, scoped grant — the network-axis equivalent of what
  `filesystem-view` does on Linux through the filesystem axis.

### Modified Capabilities

- (none — `filesystem-view`'s own spec stays Linux-scoped; this change adds a sibling
  capability rather than editing that one, since the two platforms close the same gap
  through genuinely different mechanisms, and conflating them into one spec would
  misstate what either platform actually does)

## Impact

- Affected specs: new `macos-unix-socket-mediation`.
- Affected code, as built (the spike moved this list): `src/fleet/mount.rs` and
  `src/bin/devcroft.rs` (Linux-only code gated so the crate builds on macOS at all —
  the actual prerequisite), `tests/unix_socket_not_mediated.rs` (platform-split
  assertions), `tests/abstract_socket_not_reachable.rs` (Linux-only by subject),
  `src/backend_capabilities.rs` (`pathname-unix-sockets` macOS status), and three
  user-facing surfaces that described a filesystem view and a network namespace macOS
  does not have (`doctor`, `up`'s warning, `policy --render`). **Not**
  `src/policy/capability_set.rs`: no `UnixSocketCapability` grant is compiled, per
  design.md S2. `docs/known-gaps.md` and `docs/threat-model.md` corrected only after
  the measurement, as required.
- No `fleet::mount` changes: this is deliberately not a macOS mount-view port. See
  Non-Goals.
- Depends on nothing left unfinished by `add-mount-isolation` — that change is complete;
  this one starts from its macOS-degrades-gracefully fix, already shipped.

## Non-Goals

- **Not a macOS filesystem view.** `add-mount-isolation`'s broader claim — narrowing
  what a sandbox can *see*, not just what it can reach over the network — has no macOS
  mechanism to build on (no user/mount namespaces) and is not proposed here. This change
  closes the AF_UNIX reachability gap specifically, through the network axis, because
  that is the axis macOS actually has.
- **Not a claim before a measurement.** Every finding in this proposal and its design.md
  is read from `nono` 0.74.0's source, not run. The spike is task 0, not a formality —
  see design.md's Open Question.

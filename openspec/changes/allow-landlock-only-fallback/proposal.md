## Why

On a Linux host that cannot create an unprivileged user namespace, `up`
refuses to start any sandbox — `add-mount-isolation` M4, "a view that
cannot be constructed prevents startup". That host is not exotic: Ubuntu
24.04 ships `kernel.apparmor_restrict_unprivileged_userns=1` by default,
so a stock Ubuntu desktop, and every GitHub `ubuntu-latest` runner until
this repo's CI added a sysctl step, is one. Measured on run 34823186189:
all five Linux legs failed at their first `up` with "this host cannot
create unprivileged mount namespaces". On such a host devcroft is not
degraded; it is absent. Meanwhile macOS, in the same function, takes a
warning-and-continue fallback to Seatbelt alone — and Landlock alone is
the exact boundary every Linux sandbox shipped with until 0.2.

## What Changes

- A new `up` flag, `--allow-degraded-isolation`, that lets a sandbox start
  on a Linux host without unprivileged user namespaces, confined by
  Landlock alone. Without the flag, behaviour is unchanged: `up` fails
  closed, now naming the flag as one of the remedies.
- The opt-in is a **CLI flag, never a manifest key**. A manifest is
  committed and shared; the decision to run with a weaker boundary is the
  operator's, on the host where it is weaker, and must not travel with
  the project.
- The fallback is **refused, flag or no flag, when the manifest promises
  what Landlock alone cannot keep**: a sandbox with `network.default =
  "deny"` (and so an egress proxy) is one whose rendered policy says
  ungranted unix sockets are unreachable, and on Linux without a mount
  view they are not. Landlock does not mediate AF_UNIX
  (`tests/unix_socket_not_mediated.rs`). Such a sandbox stays fail-closed.
  An `allow`-default sandbox never made that promise and may degrade.
- The degradation is **surfaced three times**, each where an operator
  would look: one warning at `up` naming the aspect, the reason, the
  residual exposure and the remedies (the sysctl, or the flag); the
  fallback **recorded in `Meta`** so `status` reports it for the
  sandbox's whole lifetime rather than recomputing from a manifest that
  cannot know; and `doctor`'s `pathname-unix-sockets` entry reporting the
  Linux status as degraded on that host.
- `exec`/`shell` auto-up does **not** take the flag. On such a host, an
  auto-up fails with the same message and the same remedy; a weaker
  boundary is asked for at `up`, explicitly, once.
- `add-mount-isolation`'s M4 is **amended, not struck**: failing closed
  remains the default and the only behaviour for deny-default sandboxes.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `filesystem-view`: "A view that cannot be constructed prevents startup"
  gains an operator opt-in, scoped to sandboxes whose network mode makes
  the fallback honest, with the degradation surfaced and recorded.
- `lifecycle`: `up` accepts `--allow-degraded-isolation`; `Meta` records
  the isolation actually applied; `status` reports it; auto-up refuses on
  such a host and names the flag.
- `cli`: the flag is on the closed command surface's `up`, listed in
  `USAGE`; `--help` shows it.
- `backend-capabilities`: `pathname-unix-sockets` on Linux reports
  `enforced` where the namespace is available and a named degradation
  where the fallback is in force.

## Impact

- `src/lifecycle/up.rs`: the `(true, false)` arm of the isolation match
  becomes a three-way decision (refuse / refuse-because-deny-default /
  degrade-with-warning), reusing the `(false, false)` arm's residual
  wording; `UpOptions` grows a field; `Meta` grows a field.
- `src/lifecycle/status.rs`, `src/bin/devcroft.rs` (`cli_up`, `USAGE`,
  `cli_status`), `src/backend_capabilities.rs`, `src/policy/degraded.rs`.
- Tests: the fail-closed test keeps passing unchanged; a new one asserts
  the flag starts an `allow`-default sandbox and *refuses* a deny-default
  one; `tests/cli_help_and_version.rs` catches the missing `USAGE` line.
  The Linux half needs a host with the namespace disabled — CI can
  provide one by skipping its sysctl step in a dedicated leg.
- Docs: `docs/known-gaps.md` (the AF_UNIX entry gains the degraded case),
  `docs/threat-model.md` (which use case the fallback backs),
  `add-mount-isolation/design.md` M4 cross-reference.

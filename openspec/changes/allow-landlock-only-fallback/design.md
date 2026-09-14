## Context

`src/lifecycle/up.rs` decides isolation in one `match` over
`(cfg!(linux), mount::probe(&exe))`:

- `(_, true)` — the view is built; Linux since `add-mount-isolation`.
- `(true, false)` — Linux, no unprivileged user namespace: `up` fails at
  layer `backend`. This is `add-mount-isolation` M4.
- `(false, false)` — macOS: one warning, then Seatbelt alone, with a
  `residual` sentence that depends on the sandbox's network mode.

The Linux refusal is measured to hit a stock Ubuntu 24.04
(`kernel.apparmor_restrict_unprivileged_userns=1` by default) and every
GitHub `ubuntu-latest` runner. M4's rationale is exact: without the view,
Landlock does not mediate AF_UNIX, so a sandbox whose policy renders
ungranted unix sockets as unreachable is lying — the nix daemon's socket
is world-accessible and the sandbox reaches it with the user's
authority. That reasoning is about the *claim*, and the claim is only
made by a sandbox that asked for network isolation.

Two invariants bound the design: "degraded capabilities are surfaced,
never silent" (CLAUDE.md), and "policy is deterministic and inspectable"
— nothing about the boundary may be true at `up` and invisible to
`status` afterwards.

## Goals / Non-Goals

**Goals:**

- A Linux host without user namespaces can run devcroft for the use case
  Landlock alone already backed (accident protection), on an explicit
  operator decision.
- The decision is impossible to make silently, impossible to make from a
  manifest, and visible for the sandbox's lifetime.
- A sandbox whose policy would lie under the fallback is still refused.

**Non-Goals:**

- Closing the AF_UNIX gap by other means on such hosts (seccomp was
  measured and rejected in `add-mount-isolation`; a setuid helper is out
  of the question for a tool that exists to *reduce* authority).
- Making the fallback the default, or reachable from `exec`/`shell`.
- Changing macOS behaviour — its branch is the template, not the subject.

## Decisions

### D1 — The opt-in is a flag on `up`, not a manifest key and not an environment variable

`up --allow-degraded-isolation`. Alternatives:

- **Manifest key** (`[sandbox] allow_degraded = true`). Rejected: a
  manifest is committed and shared; it would let one contributor's host
  limitation lower the boundary for every clone, and it would be
  invisible in a diff review as a security-relevant change. The boundary
  is a property of the host; the decision belongs to whoever runs it.
- **Environment variable** (`DEVCROFT_ALLOW_DEGRADED_ISOLATION=1`).
  Rejected: ambient, inherited by every `up` a shell ever runs, and
  exactly the shape `own-sandbox-environment` removed for the sandbox's
  own environment. CI wanting it can pass the flag in the command it
  already writes out.
- **Flag on `up` only.** Chosen. It is typed by the person who reads the
  warning, at the moment they read it.

### D2 — The fallback is refused for a deny-default sandbox, flag or no flag

The `(true, false)` arm becomes:

```
match (deny_default, opts.allow_degraded_isolation) {
    (true, _)      => Err(backend: cannot build view; this sandbox's
                       network.default = "deny" promises unix-socket
                       mediation Landlock alone cannot keep; remedy: the
                       sysctl, or network.default = "allow" for a sandbox
                       that does not need the promise)
    (false, false) => Err(backend: cannot build view; remedies: the
                       sysctl, or --allow-degraded-isolation)
    (false, true)  => warn, record, continue with Landlock alone
}
```

where `deny_default = plan.network_block || plan.network_proxy_port.is_some()`
— the same predicate the macOS arm already computes for its `residual`.

Why the network mode is the gate and not the flag alone: M4's objection
is a rendered policy that is false. An `allow`-default sandbox's policy
says nothing about unix sockets, so Landlock alone makes nothing it says
false; the AF_UNIX exposure is real but undeclared, the same as before
0.2, and the warning declares it. A deny-default sandbox's policy *does*
say it, through `network.default` and the proxy's existence. Letting a
flag override that would be "surfaced" only in the weakest sense — a
line in the terminal contradicting the durable artifact `policy
--render` prints. The refusal message names the way out that keeps the
policy honest: drop the deny, or fix the host.

Alternative: refuse only when `network.allow` is non-empty (a proxy
exists), and let a plain deny-default sandbox degrade. Rejected: a
deny-default sandbox with no allowlist is the *strictest* shape a
manifest can declare, and the one whose author would be most surprised
to learn a daemon socket was reachable.

### D3 — The degradation is recorded in `Meta`, not recomputed

`status` today derives degradations from `policy::detect_degraded(compile(manifest))`,
which is right for platform facts (`port_scoped_bind`) and wrong for a
decision taken at one `up` on one host. `Meta` gains
`isolation_fallback: Option<String>` (`"landlock-only"`), written by
`up` in the `(false, true)` arm and rendered by `status` as its own line
— `isolation: landlock-only (degraded: ungranted unix sockets reachable;
chosen with --allow-degraded-isolation)` — for as long as the sandbox
exists. `#[serde(default)]`, so a `meta.json` from an older build reads
back as no fallback, which is true of any sandbox an older build started.

`policy --render` is deliberately *not* changed: it renders the compiled
policy from the manifest, which is the same on every host; the
degradation is a property of one `up`, and `status` is where per-`up`
facts live (the proxy port is the precedent).

### D4 — `exec`/`shell` auto-up does not accept the flag

`maybe_auto_up` calls `up` with default options. On a host that needs
the fallback it fails, and the failure names `up --allow-degraded-isolation`
as the remedy. Alternative: thread the flag through `exec --allow-…`
and `shell --allow-…`. Rejected: auto-up is a convenience for the common
case, and a decision to weaken the boundary should not be one a
convenience can make; asking for it once, at `up`, is the whole point
of D1.

### D5 — `doctor` reports the host, `status` reports the sandbox

`backend_capabilities`' `pathname-unix-sockets` Linux entry gains a
probe-dependent status: `enforced` where `mount::probe` succeeds,
`EnforcedWithNamedDegradation` naming "mount namespace unavailable on
this host; with `--allow-degraded-isolation` an `allow`-default sandbox
runs under Landlock alone and ungranted unix sockets are reachable"
where it does not. The existing `linux_probe: mount_namespace_available`
is the hook. `doctor` is what the operator runs *before* `up`; the
warning at `up` and the line in `status` are the after.

### D6 — The warning reuses the macOS arm's wording, with the Linux residual

One `eprintln!`, the same shape as the `(false, false)` arm: aspect
("mount isolation"), reason ("this host cannot create unprivileged user
namespaces — on Ubuntu 24.04 that is AppArmor's default"), fallback
("confined by Landlock alone, the boundary every sandbox had before
add-mount-isolation"), residual ("a world-accessible unix socket outside
the compiled policy — the nix daemon's included — remains reachable, and
nothing narrows what the sandbox can see"), and both remedies. The
`residual` is unconditional here, unlike macOS: on Linux the deny-default
case never reaches this arm (D2), so there is only one residual to name.

## Risks / Trade-offs

- [An operator passes the flag once, forgets, and reads `policy --render`
  as the boundary] → `status` carries the fallback for the sandbox's
  lifetime (D3); `doctor` reports the host; `docs/known-gaps.md` names
  the case. The rendered policy is unchanged because it *is* unchanged —
  what differs is enforcement, and that is what `status` says.
- [A future change adds a second thing the view alone guarantees, and
  this fallback silently stops keeping it] → the gate is a predicate in
  one place (`deny_default` today); the `filesystem-view` delta says the
  fallback is admissible only where "the rendered policy makes no claim
  the fallback cannot keep", so a new claim has to update the predicate
  or fail that requirement.
- [The Linux test needs a host *without* user namespaces] → CI gets a
  dedicated leg that omits the sysctl step and runs only the fallback
  tests; locally the tests self-skip on a host where the probe succeeds,
  with the reason printed, per the suite's skip discipline.
- [CI itself starts passing the flag to get green] → the CI workflow
  keeps the sysctl step and never passes the flag outside the dedicated
  leg; the leg's job name says what it is.

## Migration Plan

No state migration: `Meta.isolation_fallback` defaults to `None` on
read. No behaviour change for any host that could run devcroft before;
the only new path is opt-in. Rollback is removing the flag, which
returns every host to M4's refusal.

## Open Questions

1. Whether `--allow-degraded-isolation` should also be accepted by `up
   --recreate` unchanged (yes — it is an `up`), and whether a later `up`
   *without* the flag on a sandbox recorded as degraded should be
   `AlreadyUp` (the keeper is healthy) or a warning that the running
   sandbox is degraded. Proposed: `AlreadyUp`, plus the `status` line;
   `up` is idempotent and should not editorialize about a decision
   already recorded.

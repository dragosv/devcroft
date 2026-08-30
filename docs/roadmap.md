# Roadmap: 0.1 to 1.0

What each release has to be true for, and why in this order. Written
2026-08-30, after an adversarial review of every open change.

This is a plan, not a promise of dates. The ordering is the argument; the
version numbers are labels for it.

## What 1.0 means

**The fleet claim holds, and every published claim is measured.** N coding
agents on one host, each with its own environment, ports, services,
resource budget and credentials, each reachable, and a blocked one
visible — with a boundary that does what `docs/threat-model.md` says it
does, on both supported platforms.

devcroft is below that today in ways that are specific rather than vague,
which is what makes the ordering derivable instead of arbitrary.

## 0.1 — a single sandbox, honestly described

**Status: implemented, unreleased.** Blocked only on things outside this
document: Zed's remote server (task 6.5, not attributed to devcroft) and
the publish itself, which needs the maintainer's accounts.

What holds today, each with a test that fails if it breaks: reproducible
environments from three closure-tier providers; a kernel-enforced boundary;
a private port table per sandbox with filtered egress inside it; services
supervised in that namespace; SSH per sandbox with VS Code and Cursor
validated; deterministic, inspectable policy.

## 0.2 — the boundary is what the documentation says

**`add-mount-isolation`** (0/21).

First because it is the only item on this list that makes a *shipped*
claim true rather than adding a new one. `tests/unix_socket_not_mediated.rs`
asserts, and passes because of, a gap: Landlock does not mediate AF_UNIX,
so every sandbox devcroft runs reaches any world-accessible unix socket —
the nix daemon's included, with `/nix` ungranted. Building further
capability on a boundary known to leak is the wrong order, and this is
also the cheapest moment to fix it, before more depends on the current
shape.

It pays a second time: fleet's task group 2 consumes the same mount plan
rather than writing one, the same way fleet already consumes
`fleet::netns`.

The hard part is not the namespace, it is the *view*. The spike that
proved the mechanism masked all of `/nix`, which closed the socket and
would also have removed the toolchain. Task group 0 measures what a real
build opens, per provider, before the plan is written.

**Also here, because it is small and unblocks judgement rather than code:**
`add-backend-capabilities` (0/26) — the offered-versus-adopted matrix for
`nono`. It has already paid for itself once, in findings that came from
reading the library rather than from the prose.

## 0.3 — one agent, end to end

**`add-agent-workload`** (0/38) and the attention half of
**`add-agent-interaction`** (0/30).

Before N agents, one has to work. Today an agent inside a devcroft sandbox
has no declared way to obtain its own tooling, and no way at all to
receive an API key — so the flagship use case does not run. That is a
viability gap, not a polish gap.

The attention half belongs here rather than in 0.4 because it is cheap
(needs no new dependency) and because fleet's per-agent record should
carry it from the start; retrofitting a state channel is more expensive
than designing one in.

The approval half can wait: its hook location is settled (the proxy
already accepts an `ApprovalBackend`), but *who implements it* is the open
question, and the fleet case is the hard one.

## 0.4 — N agents

**`add-linux-agent-fleet`** (6/56), starting with resource control.

Resource limits first, and not because they are hardest: without cgroups a
single runaway build starves every other agent, and no amount of isolation
elsewhere compensates. It is the most likely failure mode at N ≥ 3 and
currently 0/8.

Two things make fleet cheaper than its task count suggests:

- Its network half is already shipped and consumed — per-agent namespaces,
  ports, and egress all work today for the single-sandbox case.
- **D9's blocking gate may not apply.** Fleet declares that no proxy work
  starts until a seccomp notification-listener handoff is validated, on the
  reasoning that a userspace network helper makes proxy variables
  cooperative. The shipped design has no such helper — the namespace has
  loopback only and egress is a relay — so egress is already
  non-cooperative by construction. Re-derive before building either way;
  if it holds, fleet's hardest phase-0 item disappears.

**`add-port-allocation`** (3/32) resolves here rather than shipping: two
corrections in one day reduced it to hosts without user namespaces, the
host-side port mapping (which is fleet's D8), and a rare proxy-port clash.
Fold what survives into fleet and close the change.

## 0.5 — provisioning inside the boundary

**`sandbox-provisioning`** (4/37).

The largest remaining architectural change, and deliberately not earlier.
Provider resolution still runs host-side, before any boundary exists —
with one exception already closed, flox's activation hook. Everything
above works without this; nothing above is *safe* against a hostile
repository without it.

It is ordered after the fleet work because the fleet claim is what devcroft
is for, and because this change's own dependency graph is the deepest here.

## 0.6 — point it at anything

**`add-manifestless-mode`** (0/27).

Gated on 0.5 by its own proposal, correctly: it exists to be pointed at
repositories nobody has read, and evaluating a `flake.nix` executes code
from one. Shipping the tool's most exposed entry point while provisioning
still runs unconfined would aim its weakest path at its least trusted
input.

This is the adoption story — "try it on this repository" instead of
"migrate your environment" — and it is worth wanting. It is also the item
most likely to be pulled forward by impatience, which is why the ordering
argument is restated here rather than left in its proposal.

## 1.0 — measured, not argued

What separates 0.6 from 1.0 is evidence, not features:

- **macOS.** Seatbelt is implemented and has never run on a CI host. Domain
  filtering there is unverified, the mount-isolation equivalent is
  unexamined, and this project does not ship a security claim it has not
  measured. **This needs hardware the project does not have**, and is the
  most likely thing to hold 1.0 back.
- **Scale.** "Eight sandboxes cost one build" follows from a shared
  content-addressed store. It has been tested at two.
- **The published gaps.** Each entry in `docs/known-gaps.md` either closes
  or becomes a stated, permanent limitation with a reason — not a backlog
  item that quietly ages.
- **A soak.** N agents on one host for long enough that leaks, orphans and
  drift surface. Nothing in the current suite runs longer than a few
  seconds.

## Open, and deliberately unscheduled

- **`nono-proxy` adoption** (`add-egress-proxy` 4b). Proposed, not taken.
  The gap that motivated it — an unauthenticated loopback proxy — was
  closed directly. What remains is a trade for credential brokering,
  approval hooks and audit integrity against 116 crates, judged on its own
  merits whenever one of those is actually being built.
- **Seccomp** (`add-syscall-filtering`, unwritten). See D9 above: it may be
  unnecessary for egress. Broad syscall-surface reduction is a different
  argument, against kernel exploits rather than misbehaving agents, and
  belongs to whoever wants to make it.

## What this ordering optimises for

Fixing what is claimed before adding what is not (0.2), making one agent
work before many (0.3), and never shipping the most exposed entry point
before the boundary behind it is real (0.6 after 0.5).

The alternative ordering — fleet first, because it is the product — was
considered and rejected: it would put N agents on a boundary with a known
hole, and the hole is cheaper to close now than after fleet depends on the
current shape.

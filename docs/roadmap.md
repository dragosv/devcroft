# Roadmap: 0.0.1 to 1.0

What each release has to be true for, and why in this order. Written
2026-08-30, after an adversarial review of every open change.

This is a plan, not a promise of dates. The ordering is the argument; the
version numbers are labels for it.

**On the numbers, because two of them are now doing different jobs.** The
headings below label the *ordering*. The first thing actually published is
`0.0.1`, and the `0.1.0` number is held back rather than skipped:

- **Semver.** `0.0.z` is the only range cargo treats as incompatible with
  itself — to the resolver, `0.0.1` and `0.0.2` are different major
  versions. That is exactly what `src/lib.rs` already tells a reader: the
  modules are internals, published so `tests/` can drive them, with no
  stability offered. `0.1.0` would promise patch-compatibility across a
  surface nobody has curated, and dropping that promise later is the
  breaking change.
- **The claim.** The heading below this one is "the boundary is what the
  documentation says", which means today's boundary is not. Publishing a
  `0.1.0` on top of a gap the test suite asserts
  (`tests/unix_socket_not_mediated.rs`) would put the more confident number
  on the less finished thing.

So `0.1.0` is cut when the 0.2 section here lands, and the 0.0.z line runs
until then. That reads backwards as a pair of labels and forwards as a
single rule: the version claims no more than the boundary does.

## What 1.0 means

**The fleet claim holds, and every published claim is measured.** N coding
agents on one host, each with its own environment, ports, services,
resource budget and credentials, each reachable, and a blocked one
visible — with a boundary that does what `docs/threat-model.md` says it
does, on both supported platforms.

devcroft is below that today in ways that are specific rather than vague,
which is what makes the ordering derivable instead of arbitrary.

## 0.0.1 — a single sandbox, honestly described

**Status: implemented, ready to publish.** The only thing left is the
publish itself, which needs the maintainer's crates.io account — the name
is free on crates.io and on npm, checked 2026-08-31.

Zed's remote server (task 6.5) no longer blocks this. It was the stated
reason to hold the release, and it is the wrong shape for one: the failure
is Zed's forked daemon exiting without writing its own log, not attributed
to devcroft, with no CLI to drive it non-interactively. A release held on
a third party's bug is held indefinitely. It ships documented instead —
`docs/ssh-validation.md` has the matrix, and the README's gap list names
it — which is what the 0.0.z number is for.

What holds today, each with a test that fails if it breaks: reproducible
environments from three closure-tier providers; a kernel-enforced boundary;
a private port table per sandbox with filtered egress inside it; services
supervised in that namespace; SSH per sandbox with VS Code and Cursor
validated; deterministic, inspectable policy.

## 0.2 — the boundary is what the documentation says

**Status: implemented.** Both items below are done; what is left for
`0.1.0` is the publish steps themselves (roadmap correction, archiving
the two changes, cutting the version), not further code.

**`add-mount-isolation`** (21/21, done). **This is the release that gets
cut as `0.1.0`**, per the numbering rule above: it is the first point at
which the version stops out-claiming the boundary.

First because it was the only item on this list that made a *shipped*
claim true rather than adding a new one. `tests/unix_socket_not_mediated.rs`
used to assert, and pass because of, a gap: Landlock does not mediate
AF_UNIX, so every sandbox devcroft ran reached any world-accessible unix
socket — the nix daemon's included, with `/nix` ungranted. That test is
now inverted and passes because the gap is closed: every sandbox gets its
own mount namespace and filesystem view
(`fleet::mount::construct_view`), verified live through the real
`up`/`status`/`exec`/`down` CLI, not only the isolated primitive.

It paid a second time, as planned: fleet's task group 2 can now consume
the same mount plan rather than writing one, the same way fleet already
consumes `fleet::netns` — nothing in fleet has done so yet, since fleet
itself is still 6/62.

The hard part was not the namespace, it was the *view*. The spike that
proved the mechanism masked all of `/nix`, which closed the socket and
would also have removed the toolchain; task group 0's per-provider
`strace` measurement is what the real plan (`/nix/store` read-only,
`/nix/var` absent) was built from. Three real bugs surfaced only by
running the constructed view, not by review: an `EPERM` remounting a
device node read-only, an `EPERM` from a fresh `procfs` mount needing
PID-namespace ownership this change deliberately doesn't take, and an
`ENOENT` from merged-`/usr` symlinks the view didn't originally recreate
— all fixed, all recorded in that change's design.md.

A same-day adversarial review found four more issues before this landed:
`/tmp` mounting after the grants loop shadowed a nested project root
(`ENOENT`), `/tmp` ignoring its own grant mode, `policy --render`'s
`/proc` wording overclaiming a bounded view, and `up` failing closed on
every platform instead of only Linux — the last one a regression this
branch itself introduced when mount isolation briefly became
unconditional. All four fixed and tested before landing; none changed the
shape of what's described above.

**`add-backend-capabilities`** (26/26, done) — the offered-versus-adopted
matrix for `nono`, now `src/backend_capabilities.rs`, surfaced live by
`devcroft doctor`. It paid for itself twice: once in the findings the
proposal already named (credential brokering, approval hooks, audit
integrity, the proxy's missing per-session token — closed directly), and
again while writing task 1.5, which traced a claim inherited from an
earlier draft — "the abstract-unix-socket half of the AF_UNIX gap is
still open, one method call away" — and found it was already closed:
`IpcMode::SharedMemoryOnly` is `nono`'s own default, devcroft never
overrides it, and that alone requests Landlock's abstract-socket scoping
on ABI V6+. Verified live (`tests/abstract_socket_not_reachable.rs`), and
corrected everywhere the wrong claim had already propagated
(`docs/known-gaps.md`, `docs/threat-model.md`, the change's own design.md).

**`add-macos-unix-socket-scoping`** (0/11, proposed) is the same claim's
second half, and deliberately not one of the two items above. The mount
namespace that closes AF_UNIX on Linux has no Seatbelt equivalent, so
macOS was never claimed fixed by this section and does not gate
`0.1.0`'s cut — the gap there is exactly as open as it always was, still
correctly stated as open in `docs/known-gaps.md`. It does gate 1.0,
though, whose own definition below requires the boundary to hold "on
both supported platforms," which is why it is placed here rather than
left in the unscheduled list at the bottom of this document: the
mechanism is different (Seatbelt classifies unix-socket `connect()` as
network-outbound activity, so `network.default = "deny"` may already
reach it — read from `nono`'s own macOS sandbox source, not yet run),
and task group 0 is a spike confirming that live before any of it is
claimed anywhere. That spike does not need hardware this project lacks —
the maintainer has direct access to a Mac, just not through this
devcontainer — so what is missing is the run itself, not access to run
it. Nothing downstream of task group 0 starts until it reports back.

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

Three things make fleet cheaper than 6/56 suggests:

- **Its network half is already shipped** and consumed — per-agent
  namespaces, ports, and egress all work today for the single-sandbox
  case.
- **The cgroup half has a working reference.** `nono-cli`'s
  `resource_cgroup.rs` implements it, Apache-2.0, and therefore adaptable
  with attribution now that devcroft is too (`docs/prior-art.md`). It
  independently reached fleet's own D6 call — refuse rather than fall back
  — and carries four details fleet's task list was missing, including a
  post-`fork` race where the child briefly runs uncapped. The wall-clock
  timeout in the same group needs no cgroups at all and is the cheapest
  bound on an unattended agent.
- **D9's blocking gate may not apply.** Fleet declares that no proxy work
  starts until a seccomp notification-listener handoff is validated, on the
  reasoning that a userspace network helper makes proxy variables
  cooperative. The shipped design has no such helper — the namespace has
  loopback only and egress is a relay — so egress is already
  non-cooperative by construction. Re-derive before building either way;
  if it holds, fleet's hardest phase-0 item disappears, and `sandlock`
  shows a working handoff sequence if it does not.

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

**A devenv provider belongs here too, and for this change's reason rather
than its own.** devenv is closure tier and would be the fourth *nix*
provider, so it is the cheapest one left — its only open question is
criterion 4, whether its environment can be captured without running
`enterShell`. What the right answer *is* for a provider that runs a hook
flips at exactly this release: warn today, fail closed at layer `provider`
once activation is confined. Qualifying it earlier means measuring against
a promise about to change; qualifying it here means deciding once. It is
wanted before 0.6, since pointing `add-manifestless-mode` at a repository
and reporting `devenv.nix` unsupported is a poor version of "point it at
anything". See `docs/decisions.md` §1 for the entry.

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
  filtering there is unverified, and this project does not ship a security
  claim it has not measured. The AF_UNIX half of the mount-isolation
  equivalent has a scoped follow-up now (`add-macos-unix-socket-scoping`,
  0.2 above) rather than sitting here unexamined; domain filtering, and
  anything that spike turns up the proposal didn't anticipate, still need
  a real run. **This doesn't need hardware the project lacks** — the
  maintainer has direct access to a Mac, just not through this
  devcontainer — so what's missing is the run happening, not access to
  run it.
- **Scale.** "Eight sandboxes cost one build" follows from a shared
  content-addressed store. It has been tested at two.
- **The published gaps.** Each entry in `docs/known-gaps.md` either closes
  or becomes a stated, permanent limitation with a reason — not a backlog
  item that quietly ages.
- **A soak.** N agents on one host for long enough that leaks, orphans and
  drift surface. Nothing in the current suite runs longer than a few
  seconds.
- **A private disclosure process.** The README states plainly that
  `0.0.x` has none — a single-maintainer, pre-1.0 project with no
  userbase yet has nothing a formal channel would protect that a public
  issue doesn't already cover. Worth having once 1.0 actually has users:
  a GitHub security advisory (or equivalent) replaces the "just open an
  issue" line at that point, not before.

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

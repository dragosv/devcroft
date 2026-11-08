# Design — Backend Capabilities

## Context

`docs/threat-model.md` already declares this change authoritative and points at
it: "What any given backend can and cannot do ... is declared data, not prose."
The pointer has been dangling since `remove-gvisor-backend` shipped, which is
its own small demonstration of the problem — a document asserting where truth
lives, pointing at nothing.

## C1 — The matrix tracks adoption, not backends

**Decision.** With one backend, entries are keyed by *capability*, and each
records status per platform plus whether devcroft has adopted it. The original
backend-versus-backend comparison is gone.

**Rationale.** `remove-gvisor-backend` already identified this reframing and
called it "a more useful thing for it to do", which is worth agreeing with
explicitly rather than quietly implementing: a two-column matrix comparing a
backend to a deleted one is worthless, but the gap between what the sandbox
library offers and what devcroft configures is real, wide, and currently
undocumented.

Measured while writing this: devcroft sets exactly **one** of the library's
capability knobs (`set_signal_mode`). `ProcessInfoMode`, `IpcMode`, resource
limits, the snapshot/`undo` module, the keystore and the audit surface are all
left at their defaults or unused. The defaults are currently reasonable — which
is exactly why this is worth recording, because nothing anywhere states that
devcroft depends on them, and a library upgrade could change a
security-relevant default silently. `D10`'s exact-version pin in the fleet
design exists partly for this hazard; the matrix is what would make a change in
one visible.

**"Left at its default" is not itself a status, and `IpcMode` is the entry that
proves it — task 1.5.** Its default (`SharedMemoryOnly`) is what makes nono
request Landlock's abstract-unix-socket scoping; devcroft gets that enforcement
by never touching the knob, not despite never touching it. An earlier draft of
task 1.5 read the same "left at default" fact as `not-adopted` without checking
which way the default cuts, which is exactly the failure mode C2/C3 below exist
to catch — reasonable, unmeasured, and wrong. `ProcessInfoMode` is the entry
where "left at default" genuinely does mean unadopted (its default,
`Isolated`, still isolates rather than granting anything devcroft would need to
have chosen). The two knobs are in the same sentence above because both are
unset, not because both cash out the same way.

## C2 — `unverified` is a first-class status, not a caveat

**Decision.** A capability nobody has measured is `unverified`, distinct from
both `enforced` and `unsupported`.

**Rationale.** This project's recurring defect is not false claims — it is
*reasonable* claims that were never checked, and were wrong:

- Domain filtering on macOS is currently described as cooperative in
  `policy::degraded`, while the pinned library's own doc comment for the same
  path reads as enforced. Both are arguments; neither is a measurement.
- `docs/decisions.md` asserted raw sockets bypass the allowlist everywhere. A
  live test refuted it on Linux.
- A gVisor tier's port isolation was described as absent; it was actually
  present via a netns devcroft itself requested, and only absent once egress
  was granted.

Every one of those was defensible when written. A vocabulary that offers only
"works" and "doesn't" forces an unmeasured belief into one of those buckets,
which is how they got there. Making `unverified` a status a maintainer must
type is the cheapest available mechanism for keeping the distinction.

## C3 — Evidence is required, and its absence is itself a status

**Decision.** An entry names the test or measurement behind its status.
Inference does not qualify.

**Rationale.** Without this the matrix is the same prose in a table, and
degrades the same way. With it, "what is this claim based on?" has an answer
that does not require reading the implementation — and the act of writing an
entry surfaces the ones that have no answer, which is where the value is.

## C4 — The prose defers rather than summarising

**Decision.** README, `docs/threat-model.md` and `docs/decisions.md` point at
the matrix instead of restating capability claims.

**Rationale.** A summary is a copy, and copies drift — that is the whole
failure being fixed. The temptation will be to keep "a short version" in the
README for readability; the short version is what was wrong about domain
filtering for the entire period between the proxy landing and someone noticing.

What prose keeps is what a matrix cannot carry: *why* a decision was made
(`decisions.md`), what threat model applies (`threat-model.md`), and what the
project is for (README).

## Rejected Alternatives

**A support matrix.** Recording what devcroft *promises* rather than what it
*does*. Rejected: a promise cannot be verified by running a test, and the
verification requirement is what stops this becoming prose again.

**Generating the matrix from code.** Attractive, and wrong for the statuses
that matter most: `unverified` and `not-adopted` are facts about what nobody has
done, which no amount of static analysis can discover.

**Leaving it as prose but adding a review checklist.** This is roughly the
status quo — the claims are already reviewed, and drifted anyway. A checklist
depends on a reviewer knowing which of five documents restates a given claim.

## Open Questions

1. **Format and location.** TOML alongside the specs, or a Rust module the
   binary compiles in? The second makes `doctor` reporting trivial and keeps it
   from rotting silently; the first is editable without a rebuild. Leaning
   toward the second, since the requirement that changes update it in the same
   change argues for it living where the code does.

2. **Granularity.** One entry per user-visible capability ("domain filtering"),
   or per enforcement mechanism ("Landlock `NetPort` connect rule")? The first
   is what a reader wants; the second is what evidence attaches to. Possibly
   both, nested.

3. **Whether `unverified` should fail anything.** A capability nobody has
   checked could be a warning in CI, or merely visible. Erring toward visible
   at first: making it fail immediately creates pressure to mark things
   `enforced` on thin evidence, which inverts the point.

# Design — Agent Interaction

## Context

Fleet's premise is "start N agents and come back later". That premise has a
hole: an agent that stops and needs something has no way to say so, and the
operator has no way to find it except by looking at each in turn. The hole is
invisible at N=1, which is why it is not already fixed.

Two mechanisms, deliberately kept apart — see A1.

## A1 — "Stuck" is two different things and gets two different mechanisms

**Decision.** Separate *capability* interrupts (the agent hit the boundary)
from *attention* interrupts (the agent wants a judgment). Different triggers,
different responses, different implementations.

**Rationale.** They look alike from the outside — an agent that stopped — and
are nothing alike underneath:

| | capability | attention |
| --- | --- | --- |
| raised by | the kernel, via a denial | the agent, deliberately |
| answerable by | a policy decision | a person's judgment |
| devcroft's role | mediate and enforce | carry and report |
| already exists? | yes, in `nono::supervisor`, unadopted | no |

Building one mechanism for both would force the wrong shape on whichever came
second: an approval channel cannot answer "which of these two designs?", and a
message channel cannot safely grant a filesystem path.

The ordering follows from that table: attention needs nothing new and is what
fleet's agent record must account for, so it goes first. Approval adopts an
existing dependency and can follow.

## A2 — devcroft does not interpret the agent's message

**Decision.** Attention carries an opaque message. devcroft does not parse,
classify, route, or prioritise it.

**Rationale.** Any classification would be devcroft guessing about a model's
reasoning, and it would be wrong often enough to be worse than nothing —
because the failure mode is silent: a misclassified "blocking question" that
gets filed as informational is exactly the thing this change exists to stop
being lost.

There is a second reason, less obvious. A structured schema would need agents
to conform to it, which means either patching every agent or accepting that
only some can report. An opaque string is the only interface every agent can
already produce.

## A3 — The envelope is the feature; the approval prompt is not

**Decision.** Runtime capability requests are bounded by a manifest-declared
envelope of what *may be asked*. The runtime decision is only whether to grant
something already agreed to be askable.

**Rationale.** This is the decision that determines whether the feature is
useful or corrosive, and it is worth being explicit about the failure it
avoids.

An unbounded approval channel converges on "allow". The operator sees one
request at a time, out of context, usually while waiting on the agent. There is
no point at which they see the *shape* of what has been granted. After an hour
the sandbox's effective policy is the compiled policy plus an unreviewed pile
of grants, and nothing in `policy --render` says so.

Declaring the envelope moves the reviewable decision to the manifest — where it
is diffed, where it is seen whole, and where nobody is waiting on it — and
leaves the runtime question genuinely narrow.

This is the same argument `own-policy-baseline` made about baseline grants: the
problem was never that a rule was wrong, it was that 240 of them were invisible.

## A4 — Fail closed, and say which kind of closed

**Decision.** No approver, an erroring approver, or no answer within a bounded
time all deny. The agent is told *which*.

**Rationale.** Fail-closed is not in question; the library's own contract
already specifies timeout-as-denial. What this adds is the distinction, because
the two denials mean opposite things to whoever is debugging: "refused on the
merits" is a decision, "nobody was there" is an operational problem with the
fleet, and reporting both as `EACCES` guarantees the second gets diagnosed as
the first.

## A5 — Attention is orthogonal to health

**Decision.** A sandbox can be healthy and need attention simultaneously, and
both are reported.

**Rationale.** An agent waiting on a decision is working correctly. Modelling
that as a failure state trains the operator that the attention indicator is
noise — and an indicator people learn to ignore is worse than none, because it
occupies the slot a real one would have used.

## Rejected Alternatives

**A notification transport** (webhook, email, desktop). Rejected for this
change, not forever: it is a separate concern with its own configuration,
retry, and failure semantics, and the state has to exist before anything can
notify about it. `ps` reporting it is a contract anything else can poll.

**Inferring attention from process state** — noticing the agent is blocked on
stdin, say. Rejected: it cannot distinguish "waiting for a decision" from
"waiting for a slow build", it breaks the moment an agent buffers differently,
and it would make devcroft's report depend on a heuristic about someone else's
process. An explicit signal is worse ergonomics and better truth.

**Reusing the keeper's session mechanism to prompt interactively.** Attractive
— the plumbing exists — and wrong: it requires someone attached at the moment
the question arises, which is exactly the assumption fleet breaks.

## Open Questions

1. **How the agent raises attention.** A devcroft subcommand run inside the
   sandbox is the obvious shape (no protocol to learn, works over `exec` and
   SSH alike, and the binary is already reachable — the keeper-exe grant). But
   it needs a frame on the existing control socket, and the MVP command surface
   is closed, so this is post-MVP surface by construction. Decide before
   implementing; the spec deliberately says *that* an agent can raise it, not
   *how*.

2. **Whether attention should have severity or kind.** Leaning no, per A2 —
   but "blocking versus informational" is the one distinction an operator
   plausibly needs at N=20, and it is the sort of thing that is much cheaper to
   add now than to retrofit. Deferred rather than settled.

3. **Where the approval decision is actually made.** `nono`'s `ApprovalBackend`
   is a trait; something has to implement it. A prompt on a terminal assumes an
   attached operator; a file or socket assumes an external tool. This is the
   question that decides whether the approval half is usable in a fleet at all,
   and it should be answered before the half is built rather than during.

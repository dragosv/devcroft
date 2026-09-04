# Design — Reconciling Agent Workload with nono

## Context

`add-backend-capabilities` established that the gap between what a backend
*offers* and what devcroft *adopts* must be declared data rather than
folklore, and `src/backend_capabilities.rs` is that data. Twelve capabilities
currently read `not-adopted`.

`add-agent-workload` predates the library adoption that made most of them
reachable, and was never revisited on this axis. This change closes that
specific gap, not the general one.

## Goals / Non-Goals

**Goals:**
- A recorded decision per relevant capability, with the property that decides
  it — not a preference.
- `add-agent-workload` reading as though nono existed when it was written.

**Non-Goals:**
- Adopting anything; auditing capabilities fleet needs; reopening what is off
  by decision. See proposal Non-Goals.

## Decisions

## R1 — The unit of decision is a capability, and the answer is a property

Each capability gets one of three answers, and the *reason* is a property
that can be checked, not a judgement:

- **adopt** — it does what devcroft needs, and the cost is stated;
- **reject** — a named property fails (it does not enforce; it lives in a
  crate we do not link; it duplicates something devcroft already has);
- **defer with a trigger** — the answer depends on something not yet true,
  and the trigger is written down.

The third exists because two of these genuinely hang on other decisions, and
"defer" without a trigger is how a question gets re-litigated instead of
revisited — the failure `docs/decisions.md` exists to prevent.

## R2 — Credentials: the question is not "nono or not", it is "which nono"

**Measured**, and it splits the question in two:

| capability | where it lives | what that costs |
|---|---|---|
| `keystore` | the crate devcroft already links | no new dependency |
| `credential-brokering` | `nono-proxy` | the **116-crate** adoption already deferred in `add-egress-proxy` |

So "deliver the agent's API key through nono" is two different proposals.
The keystore one is cheap and open. The brokering one is the deferred
`nono-proxy` trade resurfacing under a new name, and answering it here
without saying so would smuggle a 116-crate dependency in through the agent
change.

**The complication that makes this a real decision rather than a lookup**:
devcroft already has a credential mechanism — the egress proxy's per-session
token, which `add-egress-proxy` built and which the matrix names as the
reason `keystore` has no consumer. An agent's API key is a *different* kind
of secret (it belongs to the user, not the session), so whether it belongs in
the same mechanism, in nono's keystore, or in neither is the thing to decide.

**What the threat model already constrains**: `docs/threat-model.md` states
"capability, not custody" — real secrets stay outside the sandbox and are
attached as requests cross the proxy. Any answer that puts a real API key
*inside* the sandbox contradicts a published position, whichever mechanism
carries it.

## R3 — `resource-limits`: reject, and the reason is that it does not work

nono's `ResourceLimits` is **a declaration only** — it does not enforce.
Adopting it would buy a type and no ceiling, while making `doctor` report a
capability as adopted, which is worse than reporting it unadopted.

This is the finding most worth recording, because it is the exact shape of
mistake this project has made three times (design.md C2 in
`add-backend-capabilities`: macOS domain filtering, raw-socket allowlist
bypass, gVisor port isolation — reasonable, unmeasured, wrong). The roadmap's
own answer for resource limits is cgroup v2 scope units, which is devcroft's
work regardless.

## R4 — `audit-log`: already reconciled, carry it across

The matrix entry already names a consumer — `add-agent-interaction`'s durable
record. Nothing to decide; this change only ensures `add-agent-workload` does
not independently invent one.

## R5 — `snapshot-and-undo`: defer, with the trigger written

`docs/known-gaps.md` records that an agent working in a real directory cannot
be rolled back, and names two mechanisms: per-agent git clones (fleet D7,
implemented on a branch) and snapshots, where fleet task 34 already names
nono's `undo` module as the candidate.

**Trigger**: the clone approach covers fleet and explicitly *not* the
single-agent case, because there the agent is deliberately working in the
real directory. So this becomes live exactly when single-agent rollback is
wanted — which is 0.3's territory, not fleet's, and is why it is deferred
here rather than left to fleet.

## Risks / Trade-offs

- **[Risk] Reconciliation becomes adoption by momentum.** Having asked "what
  does nono offer", the cheapest next step is to use it, and each adoption
  carries a dependency tail this project counts carefully (141 for the trust
  module, 116 for nono-proxy — both objected to, both recorded). →
  **Mitigation**: R1 requires a property, not a preference, and the
  proposal's Non-Goals put adoption in the change that needs the capability,
  with its own cost measurement.
- **[Trade-off] This change produces no code**, and a reader may reasonably
  ask why it is a change rather than a comment. Because the alternative is
  the state it is fixing: a document that silently disagrees with the
  codebase, discovered a year later by someone who assumes it was considered.

## Open Questions

1. **Where does an agent's API key actually live?** R2 frames it; it does not
   answer it. The answer has to satisfy "capability, not custody" and has to
   say what happens for subscription/OAuth auth, which `add-agent-workload`
   already establishes is file-based and cannot be served by env injection.
2. **Does adopting `keystore` alone buy anything without the broker?** If the
   key still ends up in the sandbox's environment, the keystore is storage
   with extra steps. Not measured, and it decides whether R2's cheap half is
   worth taking on its own.

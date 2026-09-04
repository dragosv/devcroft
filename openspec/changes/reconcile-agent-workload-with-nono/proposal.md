## Why

**`add-agent-workload` was written two days before devcroft adopted `nono`
as a library, and has never been reconciled with what that made available.**

Dates, from git rather than memory: `add-agent-workload` created
**2026-08-17**; `use-nono-library` created **2026-08-19**;
`add-agent-interaction` and `add-backend-capabilities` created **2026-08-29**.
The asymmetry shows in the documents — the two written after nono landed
reference it five times between them and reason about adopting
`nono::supervisor`; `add-agent-workload` mentions it **zero** times.

That is not carelessness, it is a document the world moved under. It was
touched twice since (2026-08-29, 2026-08-30) but on other axes — nono-proxy
and the roadmap — so this reconciliation never happened.

It matters because `add-agent-workload` is the change that most needs those
capabilities. It is 0.3's viability gap: an agent in a devcroft sandbox has
no declared way to obtain tooling and **no way at all to receive an API key**.
Its proposal already says credentials arrive "through the backend's
credential mechanism" — written when *backend* meant something else, and
never checked against what nono actually offers.

## What Changes

- **NEW** `agent-capability-reconciliation`: for every `nono` capability the
  matrix records as `not-adopted` and that bears on single-agent work, a
  recorded decision — adopted, or rejected with the property that fails.
  This is the discipline `add-backend-capabilities` established and
  `add-agent-interaction` already follows; `add-agent-workload` is the gap.
- `add-agent-workload`'s credential section is rewritten against what was
  measured rather than against "the backend's credential mechanism".
- **No implementation.** This change decides; the changes it feeds implement.

## Capabilities

### New Capabilities

- `agent-capability-reconciliation`: what devcroft must establish about a
  linked dependency's offerings before building something adjacent to them,
  and how the answer is recorded so it is revisited rather than re-litigated.

### Modified Capabilities

- (none — `openspec/specs/` holds no synced specs. `add-agent-workload`'s own
  artifacts are updated in place by this change, which is why it is scoped as
  reconciliation rather than as a competing proposal.)

## Impact

- **Affected artifacts**: `add-agent-workload`'s proposal and design (the
  credential section, and any assumption about "the backend"), plus the
  matrix entries that gain a named consumer.
- **Three findings already, from reading the matrix rather than assuming**,
  and two of them change the answer:
  - **`credential-brokering` is not in the crate devcroft links.** Its
    evidence reads "nono-proxy is not a dependency" — phantom/broker tokens
    live in `nono-proxy`, whose adoption is the **116-crate** decision
    already recorded and deferred in `add-egress-proxy`. So "use nono for
    credentials" is not a free adoption; it is that trade, resurfacing.
  - **`resource-limits` is a declaration only.** nono's `ResourceLimits`
    does not enforce. Adopting it would buy a type, not a ceiling — exactly
    the kind of assumption this project has shipped before and been wrong
    about.
  - **`audit-log` already names its consumer**: `add-agent-interaction`'s
    durable record. That half of the reconciliation is done and only needs
    carrying into the workload change.
- **`keystore`'s position is genuinely open**: no consumer, and devcroft
  already has its own per-session proxy token doing a related job. Whether an
  agent's API key belongs in nono's keystore or in devcroft's existing
  mechanism is the decision this change has to make rather than inherit.

## Non-Goals

- **Not adopting anything.** The output is decisions with reasons, not
  dependencies. A capability judged worth adopting becomes a task in the
  change that needs it, with its own measurement of what it costs.
- **Not revisiting what is off by decision.** TLS interception is an explicit
  non-goal, and SPIFFE and AWS routing are off by choice rather than
  omission (CLAUDE.md). Enabling any of them changes what devcroft claims and
  is not a reconciliation question.
- **Not a general audit of `nono`.** Only the capabilities that bear on one
  agent working end to end. Fleet's are fleet's.

# Tasks — Agent Interaction

Two independently shippable halves. **Attention first** — it needs no new
dependency, and fleet's agent record should carry it from the start rather than
gain it later.

## 0. Decide the shape before building

- [ ] 0.1 How an agent raises attention (design.md open question 1). A
      devcroft subcommand run inside the sandbox is the leading candidate;
      it needs a control-socket frame and lands outside the MVP's closed
      command surface, which is expected for post-MVP work but should be a
      decision rather than a drift.
- [ ] 0.2 Whether attention carries a severity or kind (open question 2).
      Cheaper now than retrofitted; leaning no, per A2.
- [ ] 0.3 Who implements `ApprovalBackend` (open question 3) — this decides
      whether the approval half works in a fleet or only with an operator
      attached. **The hook's location is settled**: `nono-proxy` accepts the
      trait directly (`start_with_approval`), and `add-egress-proxy` now
      adopts that crate, so devcroft already runs the process that would call
      it. What is unanswered is the implementation, and the fleet case is the
      hard one — a terminal prompt assumes someone is watching.

## 1. Attention: state and reporting

- [ ] 1.1 Add attention to the sandbox's recorded state: set, cleared, with an
      opaque message. Must survive the raising process exiting and not survive
      teardown.
- [ ] 1.2 The in-sandbox mechanism from 0.1, requiring no policy widening — a
      report channel that needs a manifest change to work would go unused in
      the confined sandboxes that most need it.
- [ ] 1.3 `ps` reports which sandboxes need attention.
- [ ] 1.4 `status` reports the state and the message verbatim.
- [ ] 1.5 Attention is orthogonal to health in both: a healthy sandbox needing
      attention reports both, without either presented as contradicting the
      other.

## 2. Attention: tests

- [ ] 2.1 An agent raises attention, exits, and the sandbox still reports it.
- [ ] 2.2 `ps` identifies the one sandbox needing attention among several,
      **without querying them individually** — the assertion is about the
      listing, since a per-sandbox check would pass while missing the point.
- [ ] 2.3 Raising attention succeeds under a deny-by-default policy with no
      extra grants.
- [ ] 2.4 Teardown clears it; a recreated sandbox does not start in it.
- [ ] 2.5 A failed keeper reports as failed, not as needing attention.

## 3. Approval: envelope and policy

- [ ] 3.1 Manifest surface for enabling requests and declaring the envelope.
- [ ] 3.2 Compile the envelope into the policy with an origin.
- [ ] 3.3 `policy --render` shows it, distinguishable from unconditional
      grants — a policy that can grow at runtime must be visibly different
      from one that cannot.
- [ ] 3.4 Requests outside the envelope are refused without reaching an
      approver, and the refusal names which kind of refusal it is.

## 4. Approval: mechanism

- [ ] 4.1 Adopt `nono::supervisor` for **filesystem** capability requests —
      `SupervisorListener`, `CapabilityRequest`, `ApprovalBackend`, and fd
      return via `SCM_RIGHTS`. Currently a `not-adopted` entry in
      `add-backend-capabilities`; adopting it should update that entry in the
      same change.
- [ ] 4.1b Wire an `ApprovalBackend` into the egress proxy for **network**
      endpoint decisions (`nono_proxy::start_with_approval`). Separate task
      from 4.1 on purpose: same trait, two different call sites, and the
      network one comes free with `add-egress-proxy`'s adoption while the
      filesystem one is its own piece of work.
- [ ] 4.1c Reuse the proxy's audit trail rather than inventing a second one —
      `nono`'s is append-only NDJSON with a rolling chain hash and a Merkle
      root, which is what 4.3's "durable record" should mean. devcroft's own
      proxy log is a log; this is evidence.
- [ ] 4.2 Bounded timeout, denying on expiry, on approver error, and on no
      approver — with the three distinguishable to the agent and in the record.
- [ ] 4.3 Durable record of every request, decision and reason.
- [ ] 4.4 Consider `UrlOpenRequest` for subscription/OAuth agent auth, which is
      the same channel and a real need for `add-agent-workload`'s credential
      story — but confirm it against a real agent's login flow rather than
      assuming the shapes match.

## 5. Approval: tests

- [ ] 5.1 Requests are denied when the mechanism is not enabled.
- [ ] 5.2 An unanswered request is denied, and reported as unanswered rather
      than refused.
- [ ] 5.3 A request outside the envelope never reaches the approver.
- [ ] 5.4 A granted request appears in the record with what was granted.
- [ ] 5.5 **Verify the tests fail with the mechanism disabled**, rather than
      passing because nothing was requested — the failure mode this project
      keeps finding, most recently in a brand-new test whose skip guard shared
      a probe with its assertion.

## 6. Downstream

- [ ] 6.1 `add-linux-agent-fleet`: attention becomes a field of the per-agent
      record in `agent-supervisor`, and appears in the fleet listing.
- [ ] 6.2 `add-backend-capabilities`: move `supervisor`/approval from
      `not-adopted` to its actual status, with evidence.

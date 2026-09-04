# Tasks — Reconcile Agent Workload with nono

## 1. Establish the facts, since two are already surprises

- [ ] 1.1 Confirm `credential-brokering` lives in `nono-proxy` and not in the
      crate devcroft links, and measure what adopting it would cost today —
      `add-egress-proxy` recorded 116 crates, which is worth re-measuring
      rather than quoting.
- [ ] 1.2 Confirm nono's `ResourceLimits` is a declaration that does not
      enforce. If it has gained enforcement since the matrix entry was
      written, R3's rejection is wrong and the roadmap's cgroup plan changes.
- [ ] 1.3 Establish whether `keystore` is usable without the broker, and what
      it buys if the key still reaches the sandbox's environment
      (design.md Open Question 2). A keystore whose contents end up in an env
      var is storage with extra steps.
- [ ] 1.4 Check the remaining unadopted entries for anything bearing on a
      single agent that R1–R5 did not name. The matrix is the list; the point
      is not to leave one unconsidered because nobody thought of it.

## 2. Decide, and record where the decision belongs

- [ ] 2.1 Write the per-capability outcome into `add-agent-workload`'s
      proposal — adopt, reject with the property, or defer with the trigger.
- [ ] 2.2 Rewrite its credential section against what was measured. It
      currently says credentials arrive "through the backend's credential
      mechanism", written when *backend* meant something else.
- [ ] 2.3 Answer design.md Open Question 1 — where an agent's API key lives —
      or record explicitly that it stays open and what blocks it. It has to
      satisfy `docs/threat-model.md`'s "capability, not custody", and it has
      to say what happens for the file-based OAuth case that env injection
      cannot serve.
- [ ] 2.4 Where a capability is adopted, add it as a task in the change that
      needs it, with its own cost measurement. Not here: this change decides.

## 3. Make the matrix reflect it

- [ ] 3.1 For each capability that gains a named consumer, update its
      evidence in `src/backend_capabilities.rs` — `audit-log` already names
      `add-agent-interaction`, which is the pattern.
- [ ] 3.2 For each rejection, ensure the evidence states the property rather
      than "no consumer". "No consumer" is true of an unconsidered capability
      and a rejected one alike, and the matrix should distinguish them.

## 4. Close the loop this change exists to close

- [ ] 4.1 Check whether any *other* open change predates `use-nono-library`
      (2026-08-19) and would naturally use it. `add-agent-workload` was found
      by asking; the same question has not been asked of the rest.
- [ ] 4.2 Record the chronology check itself somewhere durable, so the next
      dependency adoption prompts it rather than relying on someone
      remembering. A dependency landing mid-flight is not a one-off.

# Tasks — Policy View Fidelity

## 1. Make the degraded answer visible (design.md D1, first half)

> This half is complete on its own. After it, an in-sandbox query says "I
> cannot read this sandbox's grants" instead of an inverted verdict — the
> whole of the safety improvement, before any new artifact exists.

- [ ] 1.1 Reproduce the inversion as a test before changing anything: same
      project, same sandbox, `meta.json` made unreadable, verdict flips from
      `ALLOWED provider:flox` to `DENIED`. Without this the fix has nothing
      to prove it worked.
- [ ] 1.2 `compile_with_provider_grants` returns the policy plus its
      completeness — `Complete` | `NoSandbox` | `Unreadable { reason }` —
      instead of swallowing the distinction in one `else` arm (D2).
- [ ] 1.3 Keep `Unreadable` and a malformed `meta.json` distinct: `read_meta`
      already returns `InvalidData` for the latter, and the remedies differ.
- [ ] 1.4 `cli_why`: on `Unreadable`, report that the sandbox's recorded
      grants could not be read and why, and do **not** print a bare
      verdict as though the policy were whole.
- [ ] 1.5 `cli_policy --render`: same distinction, in whatever form suits a
      rendering rather than a verdict.
- [ ] 1.6 Confirm `NoSandbox` output is byte-identical to today's, exit code
      included. This is the common path and the likeliest regression.

## 2. Give the sandbox something to read (D1 second half, D3)

- [ ] 2.1 `up` serializes the `CompiledPolicy`, **origins included**, to
      `.devcroft/<name>/policy.json` — from the same value the
      `CapabilityPlan` is derived from, in the same operation.
- [ ] 2.2 Confirm the `CapabilityPlan` genuinely cannot serve instead —
      it drops origins, and an origin-less `why` tells an agent nothing the
      `EPERM` did not. Record the check; this is the reuse someone will
      propose later.
- [ ] 2.3 Failure to write is fatal to `up`, matching "failure to restrict is
      fatal": a sandbox whose policy cannot be inspected from within does not
      meet the invariant it was started under.
- [ ] 2.4 `why` and `policy --render` fall back to the artifact when `Meta`
      is unreadable — no flag, same command in both contexts (D4).
- [ ] 2.5 The fallback verifies the artifact belongs to the current sandbox
      instance before trusting it, and treats a mismatch as absent (D5).
      Decide what it is keyed on and say so where the code is.

## 3. Do not dirty the user's tree (D6)

- [ ] 3.1 `up` calls `ignore_artifact_dir`, not only `init`. Every sandbox
      now writes into `.devcroft/`, so the pre-existing gap widens from
      "service-using projects that skipped `init`" to all of them.
- [ ] 3.2 Verify on a project that never ran `init`: `up` leaves
      `git status` clean.

## 4. Tests

- [ ] 4.1 The inversion from 1.1 is gone: `Unreadable` reports itself.
- [ ] 4.2 No sandbox → unchanged output and exit code (teeth-check: remove
      the `NoSandbox` arm and confirm this fails).
- [ ] 4.3 **The one that matters** — from inside a real session, `why` for a
      store path returns `ALLOWED` with origin `provider:<name>`, matching
      the host's answer. Needs a live sandbox, not a unit test.
- [ ] 4.4 Render/explain agreement across all three contexts (host with
      sandbox, host without, inside): what `--render` shows granted, `why`
      reports allowed, with the same origin.
- [ ] 4.5 A stale artifact from another instance is refused, not used.
- [ ] 4.6 Skip-guard audit: 4.3 needs a real session, so it must guard on
      the capability and say what it skipped — a green run that tested
      nothing is this project's recurring failure.

## 5. Record it

- [ ] 5.1 `docs/implementation-log.md`: the measurement, and the shape of the
      bug — a fallthrough written for a legitimate case, silently covering an
      illegitimate one that only occurs where it is guaranteed to occur.
- [ ] 5.2 `docs/known-gaps.md`: remove or amend anything that claimed
      `policy --render` shows everything the backend was given. It did not,
      from inside the sandbox, and the invariant text should not have to be
      re-derived to find that out.
- [ ] 5.3 Note in `add-agent-workload` that its `why`-to-agent wiring depends
      on this change, so the dependency is recorded rather than rediscovered.

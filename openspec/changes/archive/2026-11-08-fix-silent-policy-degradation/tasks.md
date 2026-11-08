# Tasks — Policy View Fidelity

## 1. Make the degraded answer visible (design.md D1, first half)

> This half is complete on its own. After it, an in-sandbox query says "I
> cannot read this sandbox's grants" instead of an inverted verdict — the
> whole of the safety improvement, before any new artifact exists.

- [x] 1.1 Reproduce the inversion as a test before changing anything: same
      project, same sandbox, `meta.json` made unreadable, verdict flips from
      `ALLOWED provider:flox` to `DENIED`. Without this the fix has nothing
      to prove it worked.
      Reproduced live first: inside a real nix sandbox `DENIED`, exit 0; host `ALLOWED provider:nix`. **The cause on Linux is not `EPERM`:** the mount view has no state dir, so the read is "not found". `tests/policy_view_fidelity.rs` covers the unreadable case with a constructed `HOME`.
- [x] 1.2 `compile_with_provider_grants` returns the policy plus its
      completeness — `Complete` | `NoSandbox` | `Unreadable { reason }` —
      instead of swallowing the distinction in one `else` arm (D2).
      `Fidelity`: `Complete`, `NoSandbox`, `FromArtifact`, `Unreadable`, `Malformed`.
- [x] 1.3 Keep `Unreadable` and a malformed `meta.json` distinct: `read_meta`
      already returns `InvalidData` for the latter, and the remedies differ.
      `InvalidData` is `Malformed`, with its own message; tested apart.
- [x] 1.4 `cli_why`: on `Unreadable`, report that the sandbox's recorded
      grants could not be read and why, and do **not** print a bare
      verdict as though the policy were whole.
      `UNKNOWN`, the reason, exit 1. No verdict.
- [x] 1.5 `cli_policy --render`: same distinction, in whatever form suits a
      rendering rather than a verdict.
      Still renders (most of it is right), with `incomplete: <why>` on stderr and exit 1, so a stdout-and-status reader is not misled.
- [x] 1.6 Confirm `NoSandbox` output is byte-identical to today's, exit code
      included. This is the common path and the likeliest regression.
      `a_readable_record_and_no_sandbox_answer_as_before`; breaking the `NoSandbox` arm fails it.

## 2. Give the sandbox something to read (D1 second half, D3)

- [x] 2.1 `up` serializes the `CompiledPolicy`, **origins included**, to
      `.devcroft/<name>/policy.json` — from the same value the
      `CapabilityPlan` is derived from, in the same operation.
      Written beside `profile.json` from the same `compiled`, after which nothing changes it.
- [x] 2.2 Confirm the `CapabilityPlan` genuinely cannot serve instead —
      it drops origins, and an origin-less `why` tells an agent nothing the
      `EPERM` did not. Record the check; this is the reuse someone will
      propose later.
      Confirmed: the plan has no origins, and an origin is the whole of a `why` answer (design D3).
- [x] 2.3 Failure to write is fatal to `up`, matching "failure to restrict is
      fatal": a sandbox whose policy cannot be inspected from within does not
      meet the invariant it was started under.
      `?` on every write: `up` fails.
- [x] 2.4 `why` and `policy --render` fall back to the artifact when `Meta`
      is unreadable — no flag, same command in both contexts (D4).
      Inside the sandbox, read when `HOME` is that sandbox's own home (`own_sandbox_policy`); no flag. Live-tested: `why_inside_a_real_session_answers_as_the_host_does`.
- [x] 2.5 The fallback verifies the artifact belongs to the current sandbox
      instance before trusting it, and treats a mismatch as absent (D5).
      Decide what it is keyed on and say so where the code is.
      **Keyed on two things, one per context.** Inside: `HOME` being the sandbox's own home, which devcroft sets and sits beside the artifact. Host: the record's `policy_digest` (FNV-1a), written by `up` after the artifact, so a failed `up` leaves a mismatch and the host falls back to reconstructing. A leftover artifact with no record is never read on the host.

## 3. Do not dirty the user's tree (D6)

- [x] 3.1 `up` calls `ignore_artifact_dir`, not only `init`. Every sandbox
      now writes into `.devcroft/`, so the pre-existing gap widens from
      "service-using projects that skipped `init`" to all of them.
      **Done differently:** a self-ignoring `.devcroft/.gitignore` (`*`) written by `up`, as fleet's clone directory does. `ignore_artifact_dir` edits the project's own `.gitignore` and only where `.git` is at the project root, which every sample here (a subdirectory project) is not. Spec text updated.
- [x] 3.2 Verify on a project that never ran `init`: `up` leaves
      `git status` clean.
      In the live test: a git repo that never ran `init`, `git status --porcelain` empty after `up`; without the self-ignore it is dirty.

## 4. Tests

- [x] 4.1 The inversion from 1.1 is gone: `Unreadable` reports itself.
      Yes, `an_unreadable_record_says_so_instead_of_inverting_the_verdict`; mapping unreadable to no-sandbox (the old bug) fails it.
- [x] 4.2 No sandbox → unchanged output and exit code (teeth-check: remove
      the `NoSandbox` arm and confirm this fails).
      Teeth-checked as asked: breaking the `NoSandbox` arm fails two tests.
- [x] 4.3 **The one that matters** — from inside a real session, `why` for a
      store path returns `ALLOWED` with origin `provider:<name>`, matching
      the host's answer. Needs a live sandbox, not a unit test.
      `tests/policy_view_fidelity_live.rs`, real nix sandbox, the same command run inside a session through the resolved shell. Without the artifact write it reads `UNKNOWN`, exit 1.
- [x] 4.4 Render/explain agreement across all three contexts (host with
      sandbox, host without, inside): what `--render` shows granted, `why`
      reports allowed, with the same origin.
      Host and inside `policy --render` asserted byte-identical in the live test. **Found by it:** the host's reconstruction lacked the keeper binary's grant, which the backend gets; the host now prefers the digest-matched artifact. Host without a sandbox is the unchanged manifest-only path.
- [x] 4.5 A stale artifact from another instance is refused, not used.
      `inside_the_sandbox_its_own_copy_answers_and_a_leftover_one_does_not`: a leftover artifact is not read on the host.
- [x] 4.6 Skip-guard audit: 4.3 needs a real session, so it must guard on
      the capability and say what it skipped — a green run that tested
      nothing is this project's recurring failure.
      The live test guards on `backend_supported` and `host_can_build_nix_closures`, never on `nix --version`, and says what it skipped.

## 5. Record it

- [x] 5.1 `docs/implementation-log.md`: the measurement, and the shape of the
      bug — a fallthrough written for a legitimate case, silently covering an
      illegitimate one that only occurs where it is guaranteed to occur.
      `docs/implementation-log.md`, 2026-10-10.
- [x] 5.2 `docs/known-gaps.md`: remove or amend anything that claimed
      `policy --render` shows everything the backend was given. It did not,
      from inside the sandbox, and the invariant text should not have to be
      re-derived to find that out.
      Searched `known-gaps`, `threat-model` and the README for a claim that `--render` shows everything the backend gets: none to amend.
- [x] 5.3 Note in `add-agent-workload` that its `why`-to-agent wiring depends
      on this change, so the dependency is recorded rather than rediscovered.
      Already recorded there (6b, "Depends on"); 6b.1 ticked against the live test.

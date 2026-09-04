# Design — Policy View Fidelity

## Context

`why` and `policy --render` share one reconstruction path,
`compile_with_provider_grants`. It compiles the manifest, then folds in three
things only `Meta` records — provider `read_only_grants`, proxy port,
services socket — and falls through to the manifest-only answer whenever
`Meta` cannot be read.

That fallthrough is correct for the case it was written for: no sandbox
exists, so the manifest is the whole truth. It is wrong for the case nobody
wrote it for: a sandbox exists and `Meta` is unreadable. The two are one
`else` arm apart.

Measured (`--path /nix/store/…/bin/cc --op read`, same project, same
sandbox, only readability differing):

```
readable:    ALLOWED   allowed by rule provider:flox
unreadable:  DENIED    denied: not granted by any rule      exit=0
```

Inside a sandbox the second is not a failure mode, it is the only mode:
`DEVCROFT_DATA_DIR` is `filesystem_deny`/`Baseline` and not overridable
(`policy/mod.rs:252`).

## Goals / Non-Goals

**Goals:**
- No inverted verdict, in any context, silently.
- A complete answer from inside the sandbox, not merely an honest refusal.
- One compilation path still, so the two views cannot drift apart.

**Non-Goals:**
- Not relaxing the data-dir deny (proposal Non-Goals).
- Not agent wiring, not granting devcroft's binary inside the sandbox — both
  are `add-agent-workload`'s, and both sit downstream of this.

## Decisions

## D1 — Honesty and completeness are two fixes, in that order

**Decision.** First make the degraded path report itself; then remove the
need for it by writing the policy where the sandbox can read it.

**Rationale.** They have different blast radii and different failure modes,
and shipping them as one thing hides that. The honesty fix is a branch on an
existing `else` and cannot make anything worse. The artifact is a new file in
the user's working tree, with a lifecycle, a staleness question and a
gitignore consequence. If the artifact turns out to be wrong, the honesty fix
still leaves the tool truthful — which is the property that actually matters.

**Consequence worth stating:** after D1's first half alone, an agent inside a
sandbox gets "I cannot read this sandbox's grants" rather than a wrong
verdict. That is already the whole safety improvement. The artifact is what
makes the feature *useful*, not what makes it *correct*.

## D2 — Distinguish the three cases at the source, not at the call site

**Decision.** `compile_with_provider_grants` stops returning a bare
`CompiledPolicy` and returns the policy plus what it could not incorporate —
`Complete`, `NoSandbox`, or `Unreadable { reason }`. `cli_why` and
`cli_policy` each decide what to print.

**Rationale.** Two callers making the same distinction independently is how
they drift. And the distinction is genuinely the compiler's: it is the only
code that knows *which* of the three `Meta` contributions it dropped.

**Alternative rejected: make `read_meta`'s `Err` fatal.** It would fix `why`
by breaking every other reader of `Meta` that legitimately tolerates absence,
and it conflates "unreadable" with "malformed", whose remedies differ (the
spec asks for these to stay distinct).

## D3 — The artifact is the `CompiledPolicy`, not the `CapabilityPlan`

**Decision.** `up` serializes the `CompiledPolicy` — origins included — to
`.devcroft/<name>/policy.json`.

**Rationale.** The `CapabilityPlan` already crosses the exec boundary as
`DEVCROFT_CAPABILITY_PLAN` and would be the tempting reuse, but **it drops
origins**, and origins are the entire content of a `why` answer: "DENIED"
without "by rule `baseline`" tells an agent nothing it did not already know
from the `EPERM`. So this is a sibling artifact, not the same one.

**Written in the same operation that derives the plan, or not at all.** Both
come from one `CompiledPolicy` in `up`, which is what makes the spec's "cannot
disagree with what the backend was given" scenario structural rather than
tested-by-hope. Failure to write is fatal to `up`, for the same reason
failure to restrict is: a sandbox whose policy cannot be inspected from
within it does not meet the invariant it was started under.

## D4 — Fallback is transparent, not a flag

**Decision.** No `--self` mode. `why` prefers `Meta` and falls back to the
artifact; the command is the same command in both contexts.

**Rationale.** nono's pack instructs the agent to run `nono why --self`,
which works because the pack ships the instruction alongside the flag. A flag
the caller must know about is a flag the caller will not use — and the whole
point of this fix is the caller that arrives with no knowledge of where it is
running. The command that works on the host must work inside, unchanged.

## D5 — Staleness is bounded by the existing lifecycle, and checked anyway

**Decision.** The artifact is removed by `rm` along with the rest of
`.devcroft/<name>/` (`terminate.rs`), so it does not outlive its sandbox by
the normal route. The fallback still verifies the artifact belongs to the
current instance before trusting it.

**Rationale.** "Normally cleaned up" is not an invariant — a killed `rm`, a
copied working tree, a `down` that leaves artifacts in place. And a stale
policy is exactly as wrong as the degraded compile this change exists to
remove, so accepting one to fix the other would be circular. What identity is
keyed on is left to implementation; `Meta` already records `project_root` and
`env_fingerprint`, and the worktree identity check added by
`add-agent-workload` is the precedent for what a mismatch should say.

## D6 — `up` must write the ignore entry, not only `init`

**Decision.** Move `ignore_artifact_dir` so `up` calls it too.

**Why this change forces it.** Today `.devcroft/` is created only when a
project declares services, and the ignore entry is written only by `init`.
That gap is already reachable — a hand-written `devcroft.toml`, or a cloned
project — but narrow. After this change **every** sandbox writes into
`.devcroft/`, so the gap widens from "service-using projects that skipped
`init`" to "every project that skipped `init`", and the symptom is a dirty
`git status` in a file the user never created. That is precisely the
consequence `ignore_artifact_dir`'s own doc comment says devcroft is
responsible for avoiding.

## Risks / Trade-offs

- **[Risk] A second copy of the policy diverges from the first.** →
  **Mitigation**: D3's single-operation rule, plus the spec's render/explain
  agreement scenario exercised in all three contexts. The test that matters
  is the in-sandbox one, which needs a real session rather than a unit test.
- **[Risk] The honesty fix fires where it should not**, turning today's clean
  manifest-only answers into warnings for users with no sandbox — the far
  more common path. → **Mitigation**: it is the explicit second scenario in
  the spec, and worth a teeth-check that removing the `NoSandbox` arm makes
  it fail.
- **[Trade-off] Writing policy into the working tree** puts devcroft's
  internal state one directory from the user's code, where a careless
  `git add -A` in a project that skipped `init` commits it. D6 reduces this
  to a narrow window; it does not close it. The alternative — keeping it in
  the denied data dir — is what this change exists to escape.
- **[Trade-off] A sandbox can now read its own compiled policy.** Deliberate
  and, per proposal Non-Goals, discloses nothing: it describes only that
  sandbox's own grants, which its occupant can enumerate by trying. It is
  worth being explicit that this is a decision, not an oversight, because it
  is the sort of thing that reads as one later.

## Open Questions

1. **What does an in-sandbox `policy --render` render — the artifact, or
   nothing?** `--render` is an inspection command a user runs; inside the
   sandbox its audience is an agent. Whether the two want the same format is
   not obvious, and answering it wrongly means adding a format nobody reads.
2. **Does `down` remove the artifact, or only `rm`?** D5 leans on `rm`'s
   cleanup. A `down`ed sandbox is restartable, so keeping the artifact is
   arguably right — but then the identity check in D5 is the only thing
   standing between a restart and a stale answer, which raises what it must
   be keyed on.

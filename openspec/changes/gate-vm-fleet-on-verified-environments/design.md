## Context

The owner selected strict reconstruction on 2026-11-06. The gate applies to the
new VM fleet path, not the current native fleet. A project can remain useful
locally indefinitely without qualifying. No workflow may substitute an opaque
snapshot for the requested recipe just to reach fleet sooner.

## Goals / Non-Goals

**Goals:** reconstruct declared preparation in isolation, produce precise evidence,
invalidate changed inputs and admit agents only for matching verified contexts.

**Non-Goals:** prove all possible project behaviors, trust arbitrary guest reports
as security attestation, require application outputs to be bit-identical, or
infer a complete recipe from an exploratory disk.

## Decisions

### F1 — Define exactly what is reconstructed

Require a versioned environment definition with Linux target, pinned base/toolchain
and application dependency inputs, setup order, env/secret references, service
configuration/readiness/seeds, phase policy and non-empty verification commands.
Require a supported reconstruction adapter and recoverable declared inputs.
Reject known floating downloads, missing locks, undeclared ambient dependencies
and known reliance on interactive state. Arbitrary scripts cannot be statically
proven complete: verification is evidence for their declared workload, not a
universal proof. A `true` command is at most evidence that `true` ran.

Trusted base/toolchain artifacts may be pinned leaf inputs; this does not require
bootstrapping every compiler from source. Project-specific environment adaptations
must have a reconstructible recipe. Renaming an exploratory snapshot as a `base`
does not qualify it. The first image-only Dev Containers adapter is local-only
for fleet purposes until a supported reconstruction path can execute the project
environment recipe; presence of a Dockerfile is not enough when its builder is
not implemented. This is the intended consequence of the owner's two choices.

### F2 — Reconstruct in a new instance with declared inputs only

Use a separate VM, fresh private home/workspace/service data, selected source
snapshot and declared seed data. Never clone the exploratory writable disk,
copy login directories or depend on mutable caches. Permit immutable cache inputs
with verified identity/provenance. Distinguish reused materialization from actual
setup/verification execution in the evidence; a cached success is not a fresh run.

Start services, wait for declared readiness, run non-empty verification with
timeouts and fail on nonzero exit, skipped required probes or unavailable services.
Record layer/phase, command, outcome and bounded logs. Stop and clean up the
verification instance without modifying the user's local one. Only clean,
credential-free preparation outputs may become shared bases; secret-dependent
checks use disposable private verification state and cannot publish that state.

### F3 — Two identities avoid unnecessary rebuilds without stale verdicts

`environment_id` hashes a canonical versioned representation of the definition,
target, toolchain/provider versions, base/input digests, setup/service/seed data,
nonsecret environment, secret-reference contract, effective phase policy and
relevant runtime/verification compatibility version. Paths are guest-relative;
credentials are represented by reference, never raw values or value hashes.

`verification_id` additionally binds the exact source snapshot (including imported
dirty/untracked data), setup/build context, verification commands, environment_id
and agent-tooling identity. Conservatively treat every selected source change as
stale initially. Environment artifacts can be reused, but checks must rerun for
the new source. A later dependency graph optimization needs evidence of completeness.
Changes during capture or launch produce a mismatch, not best-effort reuse.

Evidence lives in controller-owned state outside project/guest write access. Store
schema, IDs, required checks and outcomes, artifact references, compatibility
version and failure reasons. An edited project `verified=true` field has no force.
Guest exits/reports are workload results, not proof of safety against a malicious
guest kernel. Credential rotation is reauthorized and validated at launch rather
than hashing secret bytes into identity.

### F4 — Admission is atomic with respect to identity and artifact leases

Before starting agent work, recompute IDs from captured immutable inputs, verify
matching successful evidence, acquire artifact leases and check their bytes,
availability and current runtime capability/policy. Check each distinct requested
agent source context; fail fleet preflight if any lacks matching evidence.
Recheck under the launch transaction to prevent a definition/cache change between
validation and use. Expose actionable states: missing-definition, incomplete-inputs,
verification-failed, stale-evidence, missing-artifact, unsupported-runtime, eligible.

Provision each agent from the accepted immutable base plus its private writable
state. Run declared per-workspace setup, seed/readiness and required startup checks
for that source; only then admit that agent's task. Another branch's result or
another agent's populated database is never inherited. Launch failures stop and
clean the failing instance and produce explicit partial-fleet status for any
already admitted siblings; they are not silently reported as full success.

Initial admission is what evidence covers. Later agent code edits do not instantly
kill a task, but the instance is now divergent and cannot supply fresh fleet
eligibility or a shared base without returning through reconstruction.

## Risks / Trade-offs

- Source-wide invalidation reruns checks often → reuse environment artifacts while
  keeping conservative verification; optimize input tracking only in a later change.
- Recipes can access mutable services → reject known unpinned material inputs and
  state external-service assumptions; do not label arbitrary setup hermetic.
- Artifact identity is not availability → lease and verify before launch; missing
  inputs fail without touching the exploratory instance.
- Guest can lie about tests → describe reported workload evidence honestly; host
  launch identity and policy checks remain outside guest control.

## Migration Plan

No existing `Meta` or native fleet status becomes verified by migration. Old
records decode as lacking VM evidence. Implement gate before enabling VM agent
dispatch and reconcile `add-linux-agent-fleet` to use private VM workspaces rather
than writable host mirrors. Rollback disables admission while preserving recipes,
evidence, local disks and exports; never falls back to native execution.

## Open Questions

Public commands and serialized schema field names can be chosen during interface
design without changing these gates. Efficient source dependency tracking and
credential-dependent shared builds remain separate work, not release shortcuts.

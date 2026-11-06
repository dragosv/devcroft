## Context

The owner selected strict reconstruction on 2026-11-06. The gate applies to the
new VM fleet path, not the current native fleet. A project can remain useful
locally indefinitely without qualifying. No workflow may substitute an opaque
snapshot for the requested recipe just to reach fleet sooner. Evidence from a
reference revision, rather than from every source, is a position from the second
review, applied when the owner asked for the fixes and not individually confirmed.

## Goals / Non-Goals

**Goals:** reconstruct declared preparation in isolation, produce precise evidence,
invalidate changed inputs, admit agents only for verified environments and keep
them usable on code that does not pass yet.

**Non-Goals:** prove all possible project behaviors, trust arbitrary guest reports
as security attestation, require application outputs to be bit-identical, gate an
agent on the health of the code it was asked to change, or infer a complete recipe
from an exploratory disk.

## Decisions

### F1 — Define exactly what is reconstructed

Require a versioned environment definition with Linux target, pinned base/toolchain
and application dependency inputs, setup order, env/secret references, service
configuration/readiness/seeds, phase policy and non-empty verification commands.
Require a supported reconstruction adapter and recoverable declared inputs.
"Pinned" means hash-pinned: every fetched input is named by content hash in the
definition or in a lockfile the adapter verifies. Finding floating downloads in
arbitrary scripts is not possible from the proxy, since CONNECT hides the URL and
TLS interception is a non-goal, so the gate does not try: F2's offline stage makes
an undeclared download fail instead. Arbitrary scripts cannot be statically
proven complete: verification is evidence for their declared workload, not a
universal proof. A `true` command is at most evidence that `true` ran.

Trusted base/toolchain artifacts may be pinned leaf inputs; this does not require
bootstrapping every compiler from source. Project-specific environment adaptations
must have a reconstructible recipe. Renaming an exploratory snapshot as a `base`
does not qualify it. The first image-only Dev Containers adapter is local-only
for fleet purposes until a supported reconstruction path can execute the project
environment recipe; presence of a Dockerfile is not enough when its builder is
not implemented. This is the intended consequence of the owner's two choices.

The first adapters are named, not implied. Closure providers qualify one at a
time, starting with the nix fixture. Image-only Dev Containers do not qualify.
Base plus recipe qualifies only once the base's package mechanism pins system
packages by content hash (`add-private-local-environments` L1). Until then the
path beyond Nix ends at local use, and a plain Python repository does not reach
fleet. That is a stated gap, not a relaxed gate.

### F2 — Reconstruct in a new instance with declared inputs only

Use a separate VM, fresh private home/workspace/service data, the reference
revision's source snapshot and declared seed data. Never clone the exploratory
writable disk, copy login directories or depend on mutable caches. Permit
immutable cache inputs
with verified identity/provenance. Distinguish reused materialization from actual
setup/verification execution in the evidence; a cached success is not a fresh run.

Reconstruction has two stages. The fetch stage runs only the adapter's fetchers,
which verify each input against its hash and run no project code. The build stage
runs setup with the instance's proxy channel closed by the host. A setup step that
reaches for a network the definition did not declare therefore fails attributably
in the clean run, instead of drifting later. Per-workspace setup at launch follows
the same two stages.

Start services, wait for declared readiness, run non-empty verification with
timeouts and fail on nonzero exit, skipped required probes or unavailable services.
Record layer/phase, command, outcome and bounded logs. Stop and clean up the
verification instance without modifying the user's local one. Only clean,
credential-free preparation outputs may become shared bases; secret-dependent
checks use disposable private verification state and cannot publish that state.

### F3 — Environment evidence is separate from the state of the code

`environment_id` hashes a canonical versioned representation of the definition,
target, toolchain/provider versions, base/input digests, the declared preparation
input set, setup/service/seed data, nonsecret environment, secret-reference
contract and relevant runtime/verification compatibility version. Paths are
guest-relative; credentials are represented by reference, never raw values or
value hashes. A provider whose input set is the whole source
(`add-vm-project-runtime` R3) has source in its `environment_id`, so its evidence
lasts only as long as that source.

Environment evidence records one clean verification run: `environment_id`, the
reference revision, the verification commands and the effective policy it ran
under. The reference revision is whatever source the user verified at. Evidence
stands for later sources while `environment_id` matches, because it says that this
environment can build and serve this project, not that a given commit is healthy.
Changing the verification commands needs a new run. Policy and agent tooling are
checked at admission rather than hashed, so widening an allowlist or updating an
agent CLI neither rebuilds nor re-verifies an environment. Changes during capture
or launch produce a mismatch, not best-effort reuse.

Per-source checks run for each agent's exact source at launch (F4). Their results
are recorded against that snapshot and never feed environment evidence.

Evidence lives in controller-owned state outside project/guest write access. Store
schema, IDs, required checks and outcomes, artifact references, compatibility
version and failure reasons. An edited project `verified=true` field has no force.
Guest exits/reports are workload results, not proof of safety against a malicious
guest kernel. Credential rotation is reauthorized and validated at launch rather
than hashing secret bytes into identity.

### F4 — Admission is atomic with respect to identity and artifact leases

Before starting agent work, recompute `environment_id` from captured immutable
inputs, verify matching successful environment evidence, acquire artifact leases
and check their bytes, availability and current runtime capability/policy. A
requested source whose inputs change `environment_id`, such as a branch that edits
the lockfile, needs environment evidence of its own; fail fleet preflight if any
source lacks it.
Recheck under the launch transaction to prevent a definition/cache change between
validation and use. Expose actionable states: missing-definition, incomplete-inputs,
verification-failed, stale-evidence, missing-artifact, unsupported-runtime, eligible.

Provision each agent from the accepted immutable base plus its private writable
state. Run declared per-workspace setup, seed/readiness and startup checks for
that source. Failures have two classes, decided by where they happen rather than
by parsing output. A setup, service readiness or artifact failure is
environment-class and that agent's task does not start. A failing per-source check
is project-class: it is recorded and attached to the task, which starts. Fixing
red code is ordinary agent work, and a gate that refused it would push users to
empty their verification commands. Another branch's result or
another agent's populated database is never inherited. Launch failures stop and
clean the failing instance and produce explicit partial-fleet status for any
already admitted siblings; they are not silently reported as full success.

Initial admission is what evidence covers. Later agent code edits do not instantly
kill a task, but the instance is now divergent and cannot supply fresh fleet
eligibility or a shared base without returning through reconstruction.

### F5 — Source and results cross through the private workspace path

Each agent's source is one snapshot per distinct requested context, captured
through the import path of `add-private-local-environments`. A context that path
cannot represent, such as a linked worktree, uses a plain-file snapshot or is
refused. Each agent's work leaves through that change's staged export, as a commit
series in a per-agent staging location tied to its source. `workspace-isolation`
integrates by fetching from the agent's clone, which here would run host Git
against guest metadata, so on this path nothing fetches and agents do not push.
Status lists which agents have undelivered results, and removing a fleet names them.

## Risks / Trade-offs

- Evidence outlives its reference revision → it speaks for the environment, not the
  code. A branch that needs a new system dependency fails its per-source check and
  returns through the definition.
- A task that must change the environment itself is not fleet work until the
  changed definition has its own evidence → do it locally, then verify.
- Providers whose input set is the whole source pay for verification per source →
  they stop paying once they can declare their inputs.
- Project-class failures start tasks on code that does not build → the failure is
  attached to the task and shown in status.
- Recipes can reach mutable services only through adapter fetchers → state remaining
  external-service assumptions; an offline build is not a claim of identical output.
- Artifact identity is not availability → lease and verify before launch; missing
  inputs fail without touching the exploratory instance.
- Guest can lie about tests → describe reported workload evidence honestly; host
  launch identity and policy checks remain outside guest control.

## Migration Plan

No existing `Meta` or native fleet status becomes verified by migration. Old
records decode as lacking VM evidence. Implement gate before enabling VM agent
dispatch and reconcile `add-linux-agent-fleet` to use private VM workspaces rather
than writable host mirrors, and F5's delivery rather than fetching from clones.
Rollback disables admission while preserving recipes,
evidence, local disks and exports; never falls back to native execution.

## Open Questions

Public commands and serialized schema field names can be chosen during interface
design without changing these gates. Input declaration for providers that cannot
enumerate theirs and credential-dependent shared builds remain separate work, not
release shortcuts. The base package mechanism that decides whether base plus
recipe qualifies is open in `add-private-local-environments`.

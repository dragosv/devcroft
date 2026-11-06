## Context

The later [local-to-fleet export](../../../local-to-fleet-workflow-2026-11-05.md)
extends the [microVM exploration](../../../devcroft-1.0-microvm-exploration-2026-11-05.md):
both closure-backed and interactively adaptable Linux environments matter.
Neither export contains runtime evidence. Upstream API descriptions are input
hypotheses, not the results of this change.

## Goals / Non-Goals

**Goals:** demonstrate a complete confined workload on both adapters, discover
integration blockers early, and compare density with equal durability semantics.

**Non-Goals:** choosing a winner from documentation, a generic VMM framework,
shipping a production backend, full Dev Containers compatibility, live snapshots,
host checkout sharing, a third libkrun 1.x or microsandbox implementation.

## Decisions

### E1 — One harness, two adapters, explicit platform rows

Use library integration in a dedicated helper process per VM. Build hvi and
libkrun 2.0 separately from the normal build. The exports' starting pins are
`351d1a15728e998486232e4815ba075a22e44f56` and
`6151710f4ea627cf0e5dbce78e4192fb4d64c2e9`, respectively. First verify that these
commits exist and expose the expected APIs; record any repin and its rationale.
Never silently substitute stable libkrun 1.x or the hvi version used by hull.

Rows: macOS ARM64, Linux ARM64 and Linux x86-64, each on real compatible
hardware. Unavailable hardware is `not-run`, never a pass or inferred support.
macOS Intel, emulation and nested virtualization are outside the experiment.
Keep CPU/RAM allocations and artifact hashes equal within each host row;
cross-host absolute measurements are not an apples-to-apples ranking.

### E2 — Minimal transport before the complete workload

Stage A boots a common kernel/initramfs, raw immutable runtime disk and private
state disk. Check READY, command I/O and exit, half-close, bounded timeouts,
cold restart and read-only enforcement. No NIC, TSI or general host socket
passthrough. hvi's reported host-initiated bridge may need a bounded reverse
stream protocol; bidirectional application traffic is a result to demonstrate.

Stage B adds the real keeper, SSH exec/shell/PTY/resize/signals, SFTP and a
guest-loopback forward. Use a pinned fixture with a real build, many small files,
Postgres or Redis readiness and persistent data. Run two instances on the same
guest service port. An agent reaching a dummy-key API rejection proves only
transport/TLS; a completed agent task is a separate test.

Stage C uses a versioned writable-base fixture: install a native dependency in
the guest, build/test, restart, then reconstruct from its saved recipe in a
fresh VM. This prevents optimizing for an immutable demo while missing local
adoption. A Nix flake is the initial closure fixture, not a claim that all
providers have qualified for Linux target resolution.

### E3 — Confinement is a gate, not a benchmark variant

Trace resource opening, thread creation, restriction and first vCPU execution.
Verify the exports' warning that libkrun `build()` may start vCPUs; a restriction
applied only before `run()` is not accepted without contrary pinned evidence.
For hvi/macOS begin with its own confinement; do not stack a second Seatbelt
initialization. Judge effective authority, including inherited descriptors,
not use of the same library name on both adapters.

Test denial of original checkout, other instances, host credentials, arbitrary
host networking and shared artifact writes. Add positive controls for the
private disk and allowed egress. Also probe malformed/oversized host–guest
frames and forged administrative requests. Guest Landlock is defense in depth;
host controls must remain meaningful if guest root bypasses guest policy.

If pre-execution confinement or bounded transport is not feasible, stop that
candidate's product qualification. Diagnostic unconfined runs are labeled and
excluded from the qualifying comparison.

### E4 — Measure the entire lifecycle

Use 1, 4 and 8 successful instances; record launch-to-READY and project-ready,
first materialization and cache reuse, build time, CPU idle, physical/logical
storage, peak host memory and memory after build/idle. Include all helpers and
proxy processes, plus builder overhead separately. Report controlled
allocate/touch/free and real-build recovery separately. Do not sum RSS as PSS.

Record kernel, VMM, guest hashes, hardware, OS, power state, cache conditions,
sample counts and raw per-run results. Default protocol: 10 launch/stop cycles
and 5 full build/idle repetitions per available configuration, with an identical
60-second idle observation window. Report median and range; small samples do
not justify tail-latency claims. Alternating adapter order reduces warm-cache
bias. Resource exhaustion at 8 is an outcome, not a row to hide.

Align disk flush/durability guarantees. The common kernel includes page reporting
so libkrun may exercise it; absence of an equivalent hvi device is recorded,
not artificially concealed. Account for image duplication across two overlapping
environments as well as reuse of one image by identical instances.

### E5 — Selection has an explicit no-go outcome

Rank correctness and isolation first, then post-build density, maintenance and
startup/I/O. Publish per-row passes, failures, skipped tests, patches, unsupported
features and costs. A conditional platform-specific recommendation is allowed;
an unmeasured platform cannot inherit it. Hard product performance thresholds
and release OS support need owner acceptance once measurements exist. Their
absence prevents a release decision, not collection of useful evidence.

## Risks / Trade-offs

- Immature APIs → pin exact revisions and retain patch inventory/reproduction.
- Guest bootstrap overwhelms the experiment → stage A fail-fast before services.
- HTTP-only egress may break a fixture → report protocol incompatibility; do not
  enable TSI/NAT to make the same result appear successful.
- Memory settings are not host-wide quotas → bound the test host's resource use
  and record helper overhead and physical disk growth.
- Negative probes are not a security audit → retain residual VMM/device/parser
  risks in the adoption report.

## Migration Plan

No production migration. Keep experimental artifacts opt-in; deleting an
experiment must not touch existing sandboxes. An accepted report unlocks the
runtime implementation. A no-go leaves the native runtime intact.

## Open Questions

Empirical: libkrun pre-vCPU confinement, hvi reverse transport, guest memory
recovery, provider profile relocation and host suspend recovery. These are
tasks to measure, not questions the owner can answer from preference.
Release decisions: target hardware budget, minimum OS versions and numerical
performance SLOs. Do not invent them to declare an experiment a winner.

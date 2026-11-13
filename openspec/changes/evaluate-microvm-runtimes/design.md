## Context

The later [local-to-fleet export](../../../local-to-fleet-workflow-2026-11-05.md)
extends the [microVM exploration](../../../devcroft-1.0-microvm-exploration-2026-11-05.md):
both closure-backed and interactively adaptable Linux environments matter.
Neither export contains runtime evidence. Upstream API descriptions are input
hypotheses, not the results of this change. Some were re-read at the pins on
2026-11-06 (adversarial review); reading code is not a result either.

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
macOS Intel and emulation are outside the experiment. Nested virtualization does
not qualify a bare-metal row, but fleet hosts are often cloud VMs, so it is run
where a host exists and reported under its own label. A missing nested row is
`not-run`, not "unsupported".
Keep CPU/RAM allocations and artifact hashes equal within each host row;
cross-host absolute measurements are not an apples-to-apples ranking.

Name where each row's regressions will run. A row the project cannot rerun, on CI
or on a named machine, cannot become a supported row later. On a supported row a
required VM test fails when it cannot run, because a skip reads as a pass.

### E2 — Minimal transport before the complete workload

Stage A boots a common kernel/initramfs, raw immutable runtime disk and private
state disk. Check READY, command I/O and exit, half-close, bounded timeouts,
cold restart and read-only enforcement. No NIC, TSI or general host socket
passthrough. hvi's `agent_sock` is host-initiated on fixed port 1024 at the pin,
so control, SSH and egress share one stream unless a second path is demonstrated.
That multiplexer and its reverse streams are hvi adapter code: count them as cost
and probe them as a host parser. Bidirectional application traffic is a result to
demonstrate. hvi accepts at most four disks; record how runtime, closure, tooling
and state fit, and what a writable base adds.

Stage B adds the real keeper, SSH exec/shell/PTY/resize/signals, SFTP and a
guest-loopback forward. Use a pinned fixture with a real build, many small files,
Postgres or Redis readiness and persistent data. Run two instances on the same
guest service port. An agent reaching a dummy-key API rejection proves only
transport/TLS; a completed agent task is a separate test.

Stage B also connects real clients, because a private workspace has no host view:
a terminal SSH client and a guest-served editor over a local forward, which are
the untrusted-project paths, and VS Code Remote-SSH as a separately labelled
trusted-project row (`add-vm-project-runtime` R8). A closure guest may lack the
FHS layout a prebuilt editor server expects; if so, that is a result to record,
not a fixture to change quietly.

Stage C uses a versioned writable-base fixture: install a native dependency in
the guest through a setup window, build/test, restart, then reconstruct from its
saved recipe in a fresh VM. The recipe pins that dependency by content hash and
reconstruction follows the fetch and offline stages of
`gate-vm-fleet-on-verified-environments` F2, so a floating input fails here too.
The fixture names the package mechanism it used; that does not choose the
product's, which stays open in `add-private-local-environments`. This prevents
optimizing for an immutable demo while missing local adoption. A Nix flake is the
initial closure fixture, not a claim that all providers have qualified for Linux
target resolution.

### E3 — Confinement is a gate, not a benchmark variant

Trace resource opening, thread creation, restriction and first vCPU execution.
At the libkrun pin, on Linux, `VmmBuilder::build()` reaches `Vmm::start_vcpus`,
which sends `VcpuEvent::Resume`, and `run()` only loops the event manager. A
restriction applied between `build()` and `run()` is therefore too late, whatever
`docs/vm-lifecycle.md` says at the same pin. On macOS `resume_vcpus` is a no-op, so
the start point there is read from `start_threaded` and traced, not assumed. hvi's
documents do not place the first guest instruction relative to its confinement
either, so it gets the same trace.
For hvi/macOS begin with its own confinement; do not stack a second Seatbelt
initialization. Judge effective authority, including inherited descriptors,
not use of the same library name on both adapters.

On a Linux host, Landlock limits what the helper can open and connect to, not
which kernel interfaces it can call: a compromised helper keeps the host's whole
syscall surface and every KVM ioctl. The Linux gate therefore also requires a
seccomp allowlist in force before the first guest instruction on every thread
that runs VMM code, installed before those threads exist or synchronized across
them, as the trace above confirms. The allowlist comes from the measured syscall
inventory of the pinned helper's build and run paths. It filters arguments where
the value decides the authority, such as ioctl request numbers and socket
families, and kills the process on a miss. The logging mode used to collect the
inventory is diagnostic. `docs/roadmap.md` leaves syscall-surface reduction to
whoever argues against kernel exploits rather than misbehaving agents, and a
hostile guest that has escaped the VMM is that argument. This is not the AF_UNIX
gap, which seccomp cannot close because it cannot read a path argument. hvi on
Linux is judged by its own filter if it ships one, as on macOS. macOS has no
seccomp; record which kernel interfaces the Seatbelt profile leaves reachable
rather than claiming parity.

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
first materialization and cache reuse, source import per instance, per-agent
setup, one clean verification run, build time, CPU idle, physical/logical
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
environments as well as reuse of one image by identical instances, and for the
per-project cache namespaces of `add-vm-project-runtime` R3, which forbid reuse
of preparation output between projects.

### E5 — Selection has an explicit no-go outcome

Rank correctness and isolation first, then post-build density, maintenance and
startup/I/O. Publish per-row passes, failures, skipped tests, patches, unsupported
features and costs. A conditional platform-specific recommendation is allowed;
an unmeasured platform cannot inherit it.

The owner records numeric no-go thresholds before density collection starts:
memory per instance after build, cold preparation, per-agent launch and storage
per environment. Stages A and B may run without them; task 4.1 does not. A
threshold changed after results exist keeps its original value in the report,
with the reason, so a result cannot quietly move its own bar. Release OS support
is decided with the same evidence.

Report every candidate against the five criteria `remove-gvisor-backend` recorded:
composition with the sandboxing core, what fleet needs, real toolchains, a
boundary its own authors will call one, and Rust library access. The fourth is
not a formality. hvi says it assumes a hostile guest and has had no external
audit; libkrun's security model makes no such claim and hands confinement to the
integrator. An unmet criterion stays visible with the owner's decision on it.
Adoption also revisits the libkrun entry in `docs/decisions.md`, whose revisit
condition this plan meets.

## Risks / Trade-offs

- Immature APIs → pin exact revisions and retain patch inventory/reproduction.
- Guest bootstrap overwhelms the experiment → stage A fail-fast before services.
- HTTP-only egress may break a fixture → report protocol incompatibility; do not
  enable TSI/NAT to make the same result appear successful.
- Memory settings are not host-wide quotas → bound the test host's resource use
  and record helper overhead and physical disk growth.
- Negative probes are not a security audit → retain residual VMM/device/parser
  risks in the adoption report.
- Helper exit zero may hide a guest panic → the native PID 1 exit probe returned
  zero through pinned libkrun/HVF, whose PSCI reset and poweroff share the same
  shutdown result. Require correlated controller shutdown intent/acknowledgement
  and helper identity before classifying an orderly stop; qualify a stop-reason
  seam separately. hvi's distinct `SystemReset` is retained in the comparison.
- Interrupted private setup may expose incomplete state → preserve the observed
  unbootable baseline and partial recipe failures, stage changes outside the
  published rootfs, and use an atomic directory exchange after syncing the
  completed recipe/marker. Five synchronized native crash points cover the
  fixed fixture; arbitrary mid-write timing, full-tree equivalence, host power
  loss and a bound on transient storage require separate qualification. The
  staging-storage follow-up records guest checkpoint use and sampled host
  allocation separately: recovery frees guest blocks without reducing the
  matched host allocation. Full preparation/common/companion storage still
  needs accounting; guest free blocks are not evidence of host reclamation.

## Migration Plan

No production migration. Keep experimental artifacts opt-in; deleting an
experiment must not touch existing sandboxes. An accepted report unlocks the
runtime implementation. A no-go leaves the native runtime intact.

## Open Questions

Empirical: libkrun pre-vCPU confinement, hvi reverse transport, guest memory
recovery, provider profile relocation and host suspend recovery. These are
tasks to measure, not questions the owner can answer from preference.
Owner decisions: target hardware budget, minimum OS versions and the numeric
thresholds of E5. Thresholds are set before density collection and are never
derived from the results to declare an experiment a winner.

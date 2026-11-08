## 1. Freeze the experiment

- [x] 1.1 Verify both upstream commits/APIs, record full pins and patch inventory; keep dependencies outside the default production build. Evidence: `experiments/microvm/fixtures/upstream.json`, separate workspace and lockfile.
- [ ] 1.2 Define the hardware matrix including a labelled nested row, fixture inputs, resource settings and raw result schema; mark unavailable rows not-run and name where each row's regressions will run.
- [ ] 1.3 Build and hash a common kernel/initramfs per architecture with required storage, vsock, Landlock and page-reporting features.
  - [x] 1.3.1 Build and hash Linux 6.12.95 for ARM64 from fixed source/config, with Landlock, virtio block/vsock/console/balloon and page reporting; boot it with the common Stage-A initramfs under libkrun.
  - [ ] 1.3.2 Build and execute the x86-64 row and demonstrate artifact reuse by hvi.
- [ ] 1.4 Build the pinned Nix closure fixture and private raw disk seed; document Linux materialization without host project tools.
- [ ] 1.5 Obtain the owner's numeric no-go thresholds and record them before task 4.1 starts.

## 2. Prove each adapter's substrate

- [ ] 2.1 Implement the isolated hvi helper with READY/stop and explicit devices; prove confinement ordering, including on Linux the syscall filter (E3).
- [ ] 2.2 Implement the isolated libkrun 2.0 helper; prove restriction before first vCPU including the build path.
  - [x] 2.2.1 Compile the pinned library helper in an independent workspace with external payload, read-only runtime disk, private locked state and plain vsock (no NIC/TSI).
  - [x] 2.2.2 Demonstrate macOS pre-build Seatbelt confinement with positive/negative checks and two real guest boots; record source trace and raw outcomes.
  - [ ] 2.2.3 Implement and demonstrate Linux pre-build confinement: Landlock plus a seccomp allowlist taken from the helper's measured syscall inventory, both in force on every VMM thread before the first vCPU, killing on a miss; prove it with forbidden-syscall probes and a working boot as the positive control.
  - [ ] 2.2.4 Qualify the required guest policy through the keeper, not direct syscalls, on a kernel with Landlock enabled.
    - [x] 2.2.4.1 Apply nono 0.77.0 before all keeper/relay threads in the ARM64 guest; demonstrate filesystem and TCP denials through real sessions with functioning positive controls. Full project-policy qualification remains open.
- [ ] 2.3 Verify transport framing, reverse streams, EOF/half-close, limits and loss behavior for both adapters, probing hvi's multiplexer as a host parser.
  - [x] 2.3.1 Record and reproducibly apply the libkrun host-EOF correction; pass macOS EOF/half-close with pending data, reconnect and oversized-frame rejection.
- [ ] 2.4 Verify read-only enforcement, persistence after cold restart, timeout cleanup and descriptor inventories.

Stage-A libkrun evidence (2026-11-07, macOS ARM64, both runs diagnostic-only):

- Baseline, `experiments/microvm/results/libkrun-macos-arm64-stage-a.json`:
  READY, ping, backend read-only enforcement, private raw state persistence,
  normal shutdown and Seatbelt probes passed. Half-close timed out on both boots
  and the kernel lacked Landlock. The failed baseline remains retained.
- Continuation, `experiments/microvm/results/libkrun-macos-arm64-landlock.json`
  (raw: `experiments/microvm/runs/20261107-174747`): two passing boots with the
  local libkrun patch (2.3.1) and the newly built Landlock ABI 6 kernel (1.3.1).
  Both passed direct guest filesystem-policy probes, half-close with 130,816
  bytes pending, frame-size refusal, persistence and shutdown, with Seatbelt
  applied before payload, devices and `VmmBuilder::build`.

Still open: the guest policy through nono/keeper rather than direct syscalls
(2.2.4), Linux host confinement with its syscall filter (2.2.3), reverse-stream
and loss cases (2.3), and the x86-64 row and hvi comparison (1.3.2, 2.1).

## 3. Exercise useful workloads and hostile inputs

- [ ] 3.1 Run real keeper exec, PTY, resize, signal and exit-status probes on both adapters.
  - [x] 3.1.1 Pass the real keeper probes on libkrun/macOS ARM64 across two cold boots: stdout/stderr, 128 KiB stdin + EOF, nonzero exit, environment/cwd, PTY resize, INT/TERM/HUP, query and disconnect cleanup. Evidence: `experiments/microvm/results/libkrun-macos-arm64-keeper.json`.
  - [ ] 3.1.2 Run the same probes through hvi and on the remaining host/architecture rows.
- [ ] 3.2 Run SSH/SFTP and guest-local forwarding with two services on the same guest port; connect a terminal client, a guest-served editor and a separately labelled Remote-SSH trusted row.
- [ ] 3.3 Run real build/small-file workloads and service readiness/persistence; label dummy-key and completed-agent checks separately.
- [ ] 3.4 Add the adaptable-base fixture, private administrative install through a setup window, restart and clean recipe reconstruction with hash-pinned inputs and an offline build stage.
- [ ] 3.5 Test checkout/sibling/credential denial and allowed access controls with confinement active.
- [ ] 3.6 Test forbidden egress, malformed frames, forged instance requests and shared-cache write attempts.
- [ ] 3.7 Exercise keeper/init/helper death, transport loss, forced shutdown and suspend/resume; inspect orphan resources.

## 4. Measure and decide

- [ ] 4.1 Collect 1/4/8-instance launch and build samples against the recorded thresholds, using the specified repetition/idle protocol and matched durability.
- [ ] 4.2 Measure full helper memory/CPU, controlled and real-build memory recovery, cold preparation, warm reuse, source import, per-agent setup and one clean verification run.
- [ ] 4.3 Measure logical/physical storage for identical and overlapping environments, per-project cache namespaces and cleanup retention.
- [ ] 4.4 Publish raw results, failures/not-run rows, integration cost, each candidate against the five recorded backend criteria and a conditional recommendation including the no-go option.
- [ ] 4.5 Obtain acceptance of the evidence, thresholds and release/platform criteria, and revisit the libkrun entry in `docs/decisions.md`, before unlocking production runtime adoption.

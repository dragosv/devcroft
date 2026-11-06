## 1. Freeze the experiment

- [ ] 1.1 Verify both upstream commits/APIs, record full pins and patch inventory; keep dependencies outside the default production build.
- [ ] 1.2 Define the hardware matrix including a labelled nested row, fixture inputs, resource settings and raw result schema; mark unavailable rows not-run and name where each row's regressions will run.
- [ ] 1.3 Build and hash a common kernel/initramfs per architecture with required storage, vsock, Landlock and page-reporting features.
- [ ] 1.4 Build the pinned Nix closure fixture and private raw disk seed; document Linux materialization without host project tools.
- [ ] 1.5 Obtain the owner's numeric no-go thresholds and record them before task 4.1 starts.

## 2. Prove each adapter's substrate

- [ ] 2.1 Implement the isolated hvi helper with READY/stop and explicit devices; prove confinement ordering.
- [ ] 2.2 Implement the isolated libkrun 2.0 helper; prove restriction before first vCPU including the build path.
- [ ] 2.3 Verify transport framing, reverse streams, EOF/half-close, limits and loss behavior for both adapters, probing hvi's multiplexer as a host parser.
- [ ] 2.4 Verify read-only enforcement, persistence after cold restart, timeout cleanup and descriptor inventories.

## 3. Exercise useful workloads and hostile inputs

- [ ] 3.1 Run real keeper exec, PTY, resize, signal and exit-status probes on both adapters.
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

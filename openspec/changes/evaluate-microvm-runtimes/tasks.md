## 1. Freeze the experiment

- [ ] 1.1 Verify both upstream commits/APIs, record full pins and patch inventory; keep dependencies outside the default production build.
- [ ] 1.2 Define the hardware matrix, fixture inputs, resource settings and raw result schema; mark unavailable rows not-run.
- [ ] 1.3 Build and hash a common kernel/initramfs per architecture with required storage, vsock, Landlock and page-reporting features.
- [ ] 1.4 Build the pinned Nix closure fixture and private raw disk seed; document Linux materialization without host project tools.

## 2. Prove each adapter's substrate

- [ ] 2.1 Implement the isolated hvi helper with READY/stop and explicit devices; prove confinement ordering.
- [ ] 2.2 Implement the isolated libkrun 2.0 helper; prove restriction before first vCPU including the build path.
- [ ] 2.3 Verify transport framing, reverse streams, EOF/half-close, limits and loss behavior for both adapters.
- [ ] 2.4 Verify read-only enforcement, persistence after cold restart, timeout cleanup and descriptor inventories.

## 3. Exercise useful workloads and hostile inputs

- [ ] 3.1 Run real keeper exec, PTY, resize, signal and exit-status probes on both adapters.
- [ ] 3.2 Run SSH/SFTP and guest-local forwarding with two services on the same guest port.
- [ ] 3.3 Run real build/small-file workloads and service readiness/persistence; label dummy-key and completed-agent checks separately.
- [ ] 3.4 Add the adaptable-base fixture, private administrative install, restart and clean recipe reconstruction.
- [ ] 3.5 Test checkout/sibling/credential denial and allowed access controls with confinement active.
- [ ] 3.6 Test forbidden egress, malformed frames, forged instance requests and shared-cache write attempts.
- [ ] 3.7 Exercise keeper/init/helper death, transport loss, forced shutdown and suspend/resume; inspect orphan resources.

## 4. Measure and decide

- [ ] 4.1 Collect 1/4/8-instance launch and build samples using the specified repetition/idle protocol and matched durability.
- [ ] 4.2 Measure full helper memory/CPU, controlled and real-build memory recovery, cold preparation and warm reuse.
- [ ] 4.3 Measure logical/physical storage for identical and overlapping environments and cleanup retention.
- [ ] 4.4 Publish raw results, failures/not-run rows, integration cost and a conditional recommendation including the no-go option.
- [ ] 4.5 Obtain acceptance of the evidence and release/platform criteria before unlocking production runtime adoption.

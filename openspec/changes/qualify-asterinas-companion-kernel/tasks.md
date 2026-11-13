## 1. Owner decision (blocks every other task)

- [ ] 1.1 The owner selects the role to qualify (companion, workload or none). Selecting the workload role renames the capability to `asterinas-guest-qualification` and adds the dependency on `add-vm-project-runtime`; a scope beyond the gates in design.md A2–A7 starts a new change.
- [ ] 1.2 Re-verify every finding in design.md A2–A6 against the Asterinas and libkrun pins in use, and record the pins and patch inventory in `experiments/microvm/fixtures/`, isolated from production Cargo builds.

## 2. Boot and device memory gates

- [ ] 2.1 Resolve ARM64 load-address alignment with the selected VMM (relocation, or the VMM's firmware boot path at `0x4000_0000`) and demonstrate two cold boots reaching the role's readiness under a confined helper on macOS ARM64.
- [ ] 2.2 Map every MMIO region with the Device attribute (`MAIR_1DEVICE`) instead of attribute index 0 in `boot_l3pt_linear_id` and in later mappings; verify from the page tables, then under sustained `virtio-blk` and `virtio-vsock` stress.
- [ ] 2.3 Either implement ARM64 PSCI bring-up in `ostd/src/arch/arm/boot/smp.rs` or cap the candidate at one vCPU and record the throughput constraint.

## 3. Memory and durability gates

- [ ] 3.1 Prove bounded memory: a guest with 512 MiB RAM reads and writes a 2 GiB dataset, sequentially and randomly, without running out of memory; implement page-cache eviction first if the test fails.
- [ ] 3.2 Run the `SIGKILL` probe on the helper during acknowledged writes; require automatic recovery and every acknowledged commit retained; record `no-go` if `ext2` loses state.

## 4. Role workload, benchmark and decision

- [ ] 4.1 Run the selected role's unmodified workload (design.md A7) under the candidate and record every ABI blocker.
- [ ] 4.2 Compare cold boot, idle memory, p50/p99 latency and `SIGKILL` recovery against Linux LTS on identical hardware rows.
- [ ] 4.3 Publish the qualification report with the patch inventory, pass/fail per gate and an explicit `go` / `no-go` per architecture row.

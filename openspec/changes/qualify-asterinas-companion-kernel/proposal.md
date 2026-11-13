## Why

**Status: deferred (owner decision, 2026-11-13).** No task starts until the owner
selects the role Asterinas would be qualified for.

The owner's 2026-11-10..12 documents, kept outside the repository, name
Asterinas as a later candidate for the **workload** kernel and keep the companion
on Linux LTS. The first draft of this change scoped Asterinas to the companion
only and attributed that scope to the 12 Nov. GOAL document. The attribution was
wrong, and that scope is not confirmed.

A source-level audit of the pinned commits found code-level blockers that apply
whichever role is chosen. This change keeps them as verified findings, so a
future qualification starts from checked facts rather than from the draft's
mistaken pins and paths.

## What Changes

- Record the change as deferred. Stage 1 (`add-vm-service-companion`, Linux LTS
  in both VMs) does not depend on it.
- Make the role an explicit owner decision. The gates are role-neutral, so this
  change carries either role; a scope beyond them is a new change.
- Record findings verified at `asterinas/asterinas` `c4b7ed57` (`v0.19.0`) and
  `containers/libkrun` `6151710f`:
  - ARM64 load address: the image is linked at `DRAM_BASE = 0x4000_0000` and
    halts in `die_load_address` when loaded elsewhere
    (`ostd/src/arch/arm/boot/bsp_boot.S`,
    `osdk/src/base_crate/aarch64.ld.template`). libkrun's direct-kernel layout
    starts DRAM at `0x8000_0000` (`src/arch/src/aarch64/layout.rs`).
  - Boot page tables map the first GiB, where the devices sit, with attribute
    index 0 (Normal write-back). The Device slot `MAIR_1DEVICE` is defined but
    not used there, and `ostd/src/arch/arm/serial.rs` carries an upstream FIXME
    asking for device memory.
  - One processor: `count_processors()` returns `Some(1)` and AP bring-up is
    `unimplemented!()` (`ostd/src/arch/arm/boot/smp.rs`).
  - Writable disk filesystems are `ext2` and `exfat` only
    (`kernel/core/src/fs/fs_impls/`).
  - `openat2` is absent from the syscall table, and there is no Landlock
    (`kernel/core/src/syscall/`).
  - Several `madvise` behaviours return `EINVAL` or do nothing, and cgroup
    memory controller attributes are unimplemented. Page-cache eviction under
    pressure is neither demonstrated nor ruled out by the sources inspected; it is
    a gate to test.
- Keep role-independent gates that any future qualification must pass: boot
  alignment, device memory attributes, bounded memory under I/O larger than RAM,
  `SIGKILL` durability, SMP or a measured one-vCPU envelope, and the role's
  unmodified workload.
- Exclude eBPF and Falco from any Asterinas guest.

## Capabilities

### New Capabilities

- `asterinas-companion-qualification`: the capability name is kept from the
  first draft, and becomes `asterinas-guest-qualification` if the owner selects
  the workload role. Its requirements are role-neutral until the owner selects a role:
  no Asterinas guest without an accepted role-specific qualification, plus the
  boot, memory-attribute, memory, durability and ABI gates.

### Modified Capabilities

None. Nothing changes in Stage 1 or the native path.

## Impact

None while deferred. A future qualification would add an experimental kernel
tree, patches and a harness under `experiments/microvm/`, isolated from the
production build. It would depend on `add-vm-service-companion` for a companion
role, or on `add-vm-project-runtime` for a workload role.

## Context

The owner deferred this change on 2026-11-13. The owner's 2026-11-10..12
documents, kept outside the repository, name Asterinas as a later hardening
candidate for the **workload** kernel and keep the companion on Linux LTS. They
give the companion a compatibility purpose: the services with the broadest Linux
ABI stay on Linux, so the new kernel does not have to serve them. The first draft
of this change inverted that, restricting Asterinas to the companion, and
attributed the inversion to the 12 Nov. GOAL document. That attribution was
wrong.

The audit read the upstream sources at `asterinas/asterinas`
`c4b7ed575b7b32773475b194d2ad41fb6d03765b` (`v0.19.0`) and
`containers/libkrun` `6151710f4ea627cf0e5dbce78e4192fb4d64c2e9`. The first draft
cited the commit `876ef980` and the paths `ostd/src/arch/aarch64/…` and
`kernel/src/…`, which do not match those sources. All paths below are at
`c4b7ed57`.

## Goals / Non-Goals

**Goals:** keep the verified blockers and the role-independent gates in one
place; make the role an explicit owner decision; prevent any Asterinas guest
without an accepted role-specific qualification.

**Non-Goals:** schedule work; choose the role; run `nono` or eBPF/Falco inside an
Asterinas guest; weaken Stage 1 durability or readiness to let a candidate pass;
infer ARM64 support from x86-64 results.

## Decisions

### A1 — The role is the owner's decision; both cases are recorded

**Workload** (the owner's stated preference): it is the most exposed kernel,
running arbitrary project code as root after setup, so implementation diversity
is worth most there. The costs are known. The guest policy the keeper applies
with `nono` (`evaluate-microvm-runtimes` 2.2.4.1) needs Landlock and `openat2`,
which are absent. Toolchains and Nix builds exercise a broad syscall surface, and
builds stress memory reclaim.

**Companion** (the first draft): the code is fixed and curated, but PostgreSQL
needs shared memory, semaphores and reliable `fsync`, which is where the ABI is
hardest. It is also the least exposed kernel, reached only through the service
protocol, and the host boundary is the VMM either way. Putting Asterinas there
reverses the compatibility reason the companion existed for.

Neither is chosen. Work on a role starts only after the owner selects it. The
gates below are role-neutral, so this change carries either role: selecting the
workload role renames the capability to `asterinas-guest-qualification` and makes
this change depend on `add-vm-project-runtime`. A scope beyond these gates is a
new change.

### A2 — Gate: boot alignment on ARM64

At the pins, libkrun's direct-kernel boot places DRAM at
`DRAM_MEM_START_KERNEL = 0x8000_0000` (`src/arch/src/aarch64/layout.rs`). Asterinas
ARM64 is not relocatable: `DRAM_BASE = 0x4000_0000`, and `_start` compares `x30`
with `_header_4` and spins in `die_load_address` on a mismatch
(`ostd/src/arch/arm/boot/bsp_boot.S`, `osdk/src/base_crate/aarch64.ld.template`).
libkrun also defines `DRAM_MEM_START_EFI = 0x4000_0000` for its firmware path.
Whether Asterinas ARM64 can boot through that path is unverified, and is an
alternative to relocating the image. The gate is two cold boots reaching the
role's readiness under a confined helper.

### A3 — Gate: device memory attributes

`bsp_boot.S` defines `MAIR_0WBWA` (Normal, write-back) and `MAIR_1DEVICE`
(Device-nGnRnE). `boot_l3pt_linear_id` maps every 1 GiB block with attribute
index 0, including the first GiB where libkrun places the GIC and the virtio-mmio
devices (`MAPPED_IO_START = 0x0a00_0000`). `ostd/src/arch/arm/serial.rs` carries
an upstream FIXME asking that the UART region be mapped as device memory. How the
virtio-mmio drivers map their registers after boot was not inspected. The gate:
every MMIO mapping uses the Device attribute, checked from the page tables, not
inferred from a smoke test.

### A4 — Gate: bounded memory under I/O larger than RAM

Several `madvise` behaviours return `EINVAL` or are deliberate no-ops
(`kernel/core/src/syscall/madvise.rs`). Cgroup memory controller attributes are
unimplemented (`kernel/core/src/fs/fs_impls/cgroupfs/controller/memory.rs`).
Neither shows whether the page cache is evicted under pressure. The gate is a
test: a guest with 512 MiB RAM sequentially and randomly reads and writes a
dataset of at least twice its RAM without OOM.

### A5 — Gate: `SIGKILL` durability on writable storage

The only disk filesystems are `ext2` and `exfat` (`kernel/core/src/fs/fs_impls/`).
The gate is the `evaluate-microvm-runtimes` 3.7.1 probe: kill the helper during
acknowledged writes and recover every acknowledged commit with no manual step.
Any pre-mount `fsck` runs under the Asterinas ABI. Stage 1's companion disk is
`ext4`, which an ext2-only kernel cannot mount, so moving an existing companion to
Asterinas needs export and restore. Rolling back to Linux works, since the ext4
driver mounts ext2.

### A6 — Gate: SMP or a measured one-vCPU envelope

`ostd/src/arch/arm/boot/smp.rs`: `count_processors()` returns `Some(1)`, and
`bringup_all_aps` is `unimplemented!()`. Either implement PSCI bring-up, or cap
the candidate at one vCPU and benchmark it against Linux LTS at one vCPU and at
the default count.

### A7 — Gate: the role's unmodified workload

For a companion role: the same catalog closure, init, `SHUTDOWN` control,
diagnostic stream and relay as `add-vm-service-companion`. For a workload role:
the keeper, SSH/SFTP, toolchains and the guest policy, whose absence would itself
fail the gate. Any missing syscall, socket option or memory semantics fails that
row.

### A8 — No eBPF or Falco in an Asterinas guest

There is no Linux `bpf()`, verifier or tracepoint infrastructure. Observability
comes from host-side channels only.

### A9 — Per-row no-go

Each row (`macOS ARM64`, `Linux ARM64`, `Linux x86-64`) is qualified on real
hardware independently. A failed gate, or a durability or latency regression
beyond owner-accepted thresholds against Linux LTS, is `no-go` for that row,
which keeps Linux LTS.

## Risks / Trade-offs

- **Passing these gates is kernel development, not qualification**: SMP bring-up,
  page-cache eviction and a journaled filesystem → treat the patch inventory and
  its maintenance as the main cost; choose `no-go` when it outweighs the benefit.
- **Upstream moves fast** → re-verify every finding against the pin in use before
  starting; the paths above are valid at `c4b7ed57` only.

## Migration Plan

None while deferred. No production configuration or default changes.

## Open Questions

- Which role, if any, the owner selects.
- The upstream timeline for ARM64 device mappings, PSCI SMP, page-cache eviction
  and a journaled filesystem.

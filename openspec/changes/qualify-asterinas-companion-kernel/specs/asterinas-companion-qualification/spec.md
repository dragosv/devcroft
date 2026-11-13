## ADDED Requirements

### Requirement: No Asterinas guest without an accepted role-specific qualification

devcroft SHALL NOT boot an Asterinas kernel for any VM role until a qualification
for that role and platform row is accepted by the owner. Stage 1
(`add-vm-service-companion`) SHALL NOT depend on this capability.

#### Scenario: Qualification is deferred

- **WHEN** a VM sandbox is launched while this change is deferred
- **THEN** both the workload and any companion boot the pinned Linux LTS kernel
- **AND** no configuration value enables an Asterinas kernel artifact

### Requirement: ARM64 boot qualification requires load-address alignment with the VMM

On ARM64, a candidate SHALL boot only at a DRAM base the selected VMM provides,
either by relocation or through a verified VMM boot path. The qualification
harness SHALL record a mismatch as a boot blocker and SHALL run no service or
workload test on that row until it is resolved.

#### Scenario: Candidate is linked for a different DRAM base

- **WHEN** a candidate linked at `0x4000_0000` is loaded by a VMM whose DRAM starts at `0x8000_0000`
- **THEN** the harness records a boot blocker for that row
- **AND** no further gate runs on that row

### Requirement: MMIO regions are mapped with Device memory attributes

A qualifying candidate SHALL map every MMIO region with a Device memory
attribute, never with normal cacheable memory, verified from its page tables.

#### Scenario: Boot page tables map the device window as Normal memory

- **WHEN** the page tables map the GIC, UART or virtio-mmio registers with the Normal write-back attribute
- **THEN** the candidate fails the ARM64 correctness gate, whether or not a short smoke test passes

### Requirement: Qualifying kernels bound memory under I/O and survive SIGKILL writes

A qualifying candidate SHALL keep guest memory bounded while reading and writing
data larger than guest RAM. It SHALL recover every acknowledged commit after the
helper receives `SIGKILL` during active writes, with no manual step.

#### Scenario: Dataset exceeds guest RAM

- **WHEN** a guest with 512 MiB RAM sequentially and randomly reads and writes 2 GiB of block data
- **THEN** memory stays bounded and the guest does not run out of memory

#### Scenario: Helper receives SIGKILL during acknowledged writes

- **WHEN** the helper is killed with `SIGKILL` while writes are in flight
- **THEN** the next boot recovers the filesystem automatically and retains every previously acknowledged commit
- **AND** unrecoverable corruption or a lost acknowledged commit makes the row `no-go`

### Requirement: The candidate runs the role's unmodified workload

A qualifying candidate SHALL run the selected role's unmodified workload: for a
companion, the Stage 1 catalog closures, init, `SHUTDOWN` control, diagnostic
stream and relay; for a workload, the keeper, SSH/SFTP, toolchains and guest
policy. It SHALL NOT require eBPF or Falco. A failing gate SHALL keep that row on
Linux LTS.

#### Scenario: A catalog service hits an unimplemented syscall

- **WHEN** an unmodified catalog closure fails readiness, migration or shutdown under the candidate
- **THEN** the failure is recorded as a kernel ABI blocker and the row remains on Linux LTS

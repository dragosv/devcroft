## Why

The current process runtime deliberately does not support hostile projects.
Moving only agent sessions into a VM would leave provider evaluation, builds,
cache publication and host grants outside the proposed boundary.

## What Changes

- Add a separately identified Linux guest runtime, gated on
  `evaluate-microvm-runtimes`, with one confined helper per VM.
- Put all project-controlled evaluation, materialization, setup and execution
  inside an appropriate VM boundary, including on macOS.
- Keep keeper and SSH/SFTP/forwarding in the guest; retain host-side controller,
  lifecycle state and narrowly scoped egress enforcement.
- Separate project requests from user-owned authority; inspect host descriptors,
  devices, guest grants and phase-specific channels as one launch plan.
- Use immutable verified runtime artifacts and private writable disks; distinguish
  guest health, keeper health, service readiness and forced shutdown.
- **BREAKING for the future VM path:** host provider tools, host library grants,
  implicit credential forwarding and process-tier fallback are not accepted.
  Existing native instances retain their current contract.

## Capabilities

### New Capabilities

- `vm-project-runtime`: boot, guest sessions, state, persistence, recovery and
  explicit platform/compatibility behavior.
- `vm-authority-boundary`: provisioning isolation, trusted bootstrap, effective
  grants, artifact publication and non-bypassable host channels.

### Modified Capabilities

None in this additive proposal. Existing lifecycle/provider/policy/SSH deltas
describe the native path; changes to defaults require the migration review in
the design. These VM-specific requirements must not be applied retroactively
to the current process runtime.

## Impact

`src/lifecycle`, `src/provider`, `src/keeper`, `src/ssh`, `src/proxy`,
`src/backend_capabilities.rs`, distribution and persistent state schemas.
Depends on an accepted experimental result, not merely completion of spike
tasks. Enables `add-private-local-environments` and the VM fleet gate.

The runtime will need an explicit Linux guest contract; Darwin/Xcode workloads
remain native and must not silently select this path. No unsupported-host
downgrade may retain a VM isolation claim.

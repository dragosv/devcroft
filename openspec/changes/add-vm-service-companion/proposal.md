## Why

In `add-vm-project-runtime`, creation-time setup runs inside the workload VM
with guest root (`R5`), after which that guest is one trust domain. Running
stateful backing services (PostgreSQL, Redis) inside the workload VM leaves
their memory, binaries, configuration, administrative identities and raw disk
exposed to any root process in the workload, and ties service survival and clean
database shutdown to workload crashes or OOM. The data the application's own
client role can reach stays reachable either way; design C9 states that limit.
The shared Lima VM in `add-macos-service-vm` serves only native macOS sandboxes
and does not give VM sandboxes per-instance kernel and disk isolation.

## What Changes

- Launch a private per-sandbox companion microVM (`VM companion`) paired with
  the workload microVM (`VM workload`) whenever a VM sandbox requests one or
  more profiles from the trusted service catalog by name; launch no companion VM
  when a sandbox requests no catalog profile.
- Run only verified catalog profiles in the companion, never provider-declared
  service commands (`ServiceDecl.command`), which are project shell code and
  remain in the workload.
- Run the pinned Linux LTS kernel in both VMs under the VMM selected by
  `evaluate-microvm-runtimes` task 4.5. Choose the companion guest base (NixOS
  minimal vs. pinned closure + dedicated init) by measurement on the same
  fixture (task 1.4).
- Give the companion zero outbound egress, zero shell, no SSH, no keeper and no
  workspace mount. Keep its host channels to a closed, host-initiated inventory:
  per-port service relay mappings, a single-opcode `SHUTDOWN` control channel and
  a diagnostic stream (owner-approved exception, 2026-11-13). PID 1 accepts
  control and diagnostics only from the host and reserves diagnostic capacity
  for its own events.
- Rely on the per-instance broker for instance identity instead of delivering
  per-instance secrets, and make every server identity other than the
  workload's least-privilege client role unreachable through the relay.
- Relay each declared port through a workload loopback listener, one broker
  stream per TCP connection and a companion forwarder, with a finite connection
  cap.
- Probe readiness from the broker before workload creation-time setup and before
  admitting interactive or fleet sessions, failing at layer `keeper` (exit 5) on
  timeout, and map every other companion failure to an existing layer.
- Close relay streams before sending `SHUTDOWN`, stop PostgreSQL with `SIGINT`
  (Fast Shutdown), and classify a stop as orderly only on PID 1's acknowledgement,
  never on the helper's exit status. Restart a crashed companion automatically
  (owner decision, 2026-11-13) from a per-instance companion supervisor, always
  through disk check and service recovery, with bounded retries and no restart
  for failures a restart cannot fix.
- Fix durability settings in each profile (Redis `appendfsync always`,
  PostgreSQL synchronous commit), give `companion-rw.img` a finite size, use full
  flush semantics, and bind persistent data to its data format version.
- Confine each VMM helper and the broker in-process before the first guest
  instruction or stream, including which Unix sockets each role can reach:
  Seatbelt on macOS; Landlock ABI ≥ 6, a `seccompiler` syscall allowlist and a
  per-role mount namespace (or separate host UID) on Linux.
- State the companion's protection claim together with its limits, and account
  for both helpers, both guests, the broker and `companion-rw.img` in launch
  plans, `status` and `2N` fleet admission without reclassifying retained
  builder artifacts.

## Capabilities

### New Capabilities

- `vm-service-companion`: conditional companion launch, trusted catalog
  profiles, loopback relay endpoints and broker, readiness probes, error layers,
  graceful stop and stop classification, automatic restart, durability, disk
  size and data format version binding, and dual-VM resource accounting.
- `companion-authority-boundary`: zero-egress and zero-shell guest, closed
  host-initiated `SHUTDOWN` and diagnostic channels, broker-derived identity with
  only the client role reachable, explicit protection limits and per-role host
  confinement of Unix-socket reach.

### Modified Capabilities

None in the native path. `openspec/specs/` is empty, so in the unarchived VM
chain this change names the requirements it amends when a VM sandbox requests
catalog profiles; they are reconciled at first archive (task 7.1):

- `add-vm-project-runtime`: "Every host channel is bounded and instance-scoped"
  (the workload gains a relay channel), "Resource budgets include host-side
  support processes" (second helper, broker and companion supervisor, a
  per-instance process with the instance's lifetime), "Stop preserves data and forced
  stop is reported as a crash" (companion stop order and acknowledgement-based
  classification) and "Removal and recovery respect ownership and compatibility"
  (`companion-rw.img` is an owned resource).
- `add-private-local-environments`: "Local work survives stop independently of
  the original checkout" (service data lives on `companion-rw.img`) and
  "Destructive operations name unexported work" (companion data).
- `gate-vm-fleet-on-verified-environments`: the `verified-fleet-environment`
  `2N` admission budget and a fresh per-agent `companion-rw.img`.
- The `devcroft.toml` schema gains a key naming catalog profiles (spelling
  decided in task 1.3).

## Impact

`src/lifecycle`, `src/services`, `src/provider`, `src/config`, the workload guest
image (relay listener), VMM helper and broker launchers, catalog profile
definitions, persistent state and fleet admission. Depends on
`evaluate-microvm-runtimes` (including the VMM selection and
`author_claimed_security_boundary` decision in task 4.5) and
`add-vm-project-runtime`. Stage 1 does not depend on
`qualify-asterinas-companion-kernel`, which is deferred.

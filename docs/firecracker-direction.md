# Firecracker Runtime Direction

Status: architectural exploration. Nothing in this document is implemented or
part of the current devcroft release. A real implementation requires its own
OpenSpec change, measurement gates and an explicit revision of the threat model.

The exploration was drafted externally (2026-09-15) and is kept verbatim;
devcroft's own reading of it — what it gets right, what it does not position
itself against, and the conditions under which it should proceed — is the
**Review** section at the end. Read that before acting on anything above it.

## Summary

Firecracker is a strong candidate for an optional Linux microVM runtime. It is
a better fit than restoring the removed gVisor tier because it provides the
thing that tier could not justify: a hardware-virtualized boundary with a
private guest kernel, process space, network stack and port table.

The proposed product would have two execution runtimes under one control plane:

```text
                       devcroft
                          |
             +------------+------------+
             |                         |
             v                         v
       process runtime          Firecracker runtime
       native and fast          stronger Linux boundary
             |                         |
             +------------+------------+
                          |
                    nono policy
                          |
        environment + services + SSH + inspection
```

The value is not merely adding Firecracker. Many projects already use it. The
potential differentiator is preserving the same provider environment, policy,
services, SSH identity and CLI while allowing the user to select the runtime
appropriate to the workload.

The first Firecracker deliverable should be a private clone inside the guest.
Sharing the host worktree should be a separate, weaker mode implemented only
after Firecracker has stable filesystem-sharing support suitable for it.

## Product Position

The current process runtime remains the default for:

- repositories the user already trusts;
- fast local build and test loops;
- native Linux development;
- native macOS and Swift development;
- accidental blast-radius reduction.

The Firecracker runtime would serve:

- unreviewed repositories and external pull requests;
- agents that need a stronger boundary than Landlock or Seatbelt;
- private workspaces that must not modify the host checkout directly;
- Linux agent fleets requiring one kernel and resource budget per agent.

This changes one part of devcroft's positioning. The accurate statement would
be:

> Process mode is native and container/VM-free. An optional Firecracker mode
> provides a stronger Linux boundary for workloads that need it.

Firecracker would not replace process mode. In particular, it cannot provide a
native macOS execution environment for Xcode, Apple SDKs, codesigning or
Darwin-specific behavior.

## Runtime And Workspace Axes

A single `hardened` boolean would hide the most important distinction: whether
the guest receives the real host worktree.

The model should expose runtime and workspace separately:

| Runtime | Workspace | Property |
|---|---|---|
| `process` | `host` | Native process tree, fastest loop, accident containment |
| `firecracker` | `clone` | Private guest disk, strongest proposed boundary |
| `firecracker` | `shared` | Host worktree visible through a filesystem server, interactive but weaker |

Possible configuration:

```toml
[sandbox]
runtime = "process"
workspace = "host"
```

```toml
[sandbox]
runtime = "firecracker"
workspace = "clone"
```

```toml
[sandbox]
runtime = "firecracker"
workspace = "shared"
```

The runtime must be selected explicitly and must fail closed. A host unable to
provide the requested Firecracker guarantees must not silently fall back to the
process runtime.

## Why Firecracker Is A Better Fit Than gVisor

The removed gVisor backend was built and tested, then removed for structural
reasons:

- Landlock could not be applied before `runsc`, because `runsc` needed
  `mount()` and an active Landlock ruleset denied it;
- rootless network behavior did not provide the complete fleet properties the
  tier needed;
- the tier occupied a squeezed middle between a cheaper process policy and a
  stronger VM;
- each feature created a second lifecycle and policy projection to maintain.

Firecracker changes the third point instead of repeating it: it is the stronger
VM option. Each microVM has its own guest kernel and therefore its own process,
IPC and network namespaces by construction.

Firecracker also offers primitives directly relevant to devcroft:

- KVM isolation on Linux `x86_64` and `aarch64`;
- a deliberately small device model;
- built-in seccomp filters for the VMM;
- configurable vCPU and memory allocation;
- block-device and network rate limiting;
- cgroup integration through the jailer;
- virtio-vsock for host/guest communication;
- snapshots and demand-paged restore;
- boot-time overhead designed for high-density microVMs.

These are Firecracker properties, not yet devcroft properties. They must be
measured on ordinary development hardware before devcroft makes performance or
density claims based on them.

## Layering With nono

Firecracker would not replace nono. The two enforce different boundaries:

```text
nono
  decides which resources the workload may use

Firecracker
  limits the result if the workload or guest kernel escapes that policy
```

The recommended layering is:

```text
host
  |
  +-- jailer / nono constrains VMM and host helpers
  |
  +-- Firecracker
        |
        +-- guest Linux
              |
              +-- nono constrains the agent
                    |
                    +-- commands
                    +-- services
                    +-- tools
```

### nono in the guest

Without a guest policy, a process that is safely isolated from the host still
has unrestricted access to everything placed in the guest:

- the root filesystem;
- workspace and private disks;
- the provider closure;
- guest-local service state;
- credentials delivered into the guest;
- every exposed vsock endpoint.

Running nono in the guest preserves the existing policy semantics:

```text
/workspace       read-write
/nix/store       read-only
/home/devcroft   read-write
/run/devcroft    narrowly scoped
everything else  denied unless declared
```

If a workload escapes nono or the guest Landlock policy, it obtains more guest
authority but remains behind the Firecracker boundary.

### nono on the host

In a development-oriented launcher, nono can constrain the Firecracker VMM and
its helpers to:

- `/dev/kvm`;
- the selected guest kernel;
- rootfs and attached disk images;
- the Firecracker API socket;
- vsock backing sockets;
- bounded log and metric files;
- no arbitrary host network access.

This is useful defense in depth against a VMM escape. It is not a replacement
for Firecracker's official jailer in a production-grade boundary claim.

### nono around filesystem sharing

For a shared host worktree, the filesystem server performs host syscalls on
behalf of the guest. That process is part of the security boundary and must be
restricted independently to exactly one worktree. Restricting only the
Firecracker process is insufficient when `virtiofsd` or another vhost-user
process opens host paths itself.

## Communication Model

devcroft would use two separate channels:

1. Firecracker's HTTP API over a host Unix socket for VMM configuration and
   lifecycle.
2. virtio-vsock for all communication with the guest runtime.

```text
                              HOST
+----------------------------------------------------------------+
| devcroft CLI / supervisor                                     |
|                                                                |
| Firecracker API client ------ firecracker-api.sock             |
|          |                         |                            |
|          | configure/start        v                            |
|          |                  Firecracker VMM                    |
|          |                         |                            |
| keeper protocol -----------+      | virtio-vsock               |
| SSH ProxyCommand ----------+------+------------------------+   |
| egress proxy <-------------+      |                        |   |
+-----------------------------------+------------------------+---+
                                    |                        |
                              GUEST LINUX                    |
                         +----------+------------------------+---+
                         | devcroft-init                         |
                         |   +-- keeper                          |
                         |       +-- control                     |
                         |       +-- embedded SSH                |
                         |       +-- sessions                    |
                         |       +-- services                    |
                         +---------------------------------------+
```

### Firecracker API

Each microVM receives a private API socket under its state directory:

```text
~/.local/state/devcroft/<name>/firecracker-api.sock
```

devcroft should use a small typed Rust client speaking HTTP over `UnixStream`,
not shell out to `curl`.

The startup sequence is approximately:

```text
spawn Firecracker
    |
    +-- wait for firecracker-api.sock
    +-- configure logger and metrics
    +-- PUT /machine-config
    +-- PUT /boot-source
    +-- PUT /drives/rootfs
    +-- PUT /drives/runtime
    +-- PUT /drives/workspace
    +-- PUT /drives/state
    +-- PUT /vsock
    +-- PUT /actions { InstanceStart }
    +-- wait for guest READY over vsock
```

An API response to `InstanceStart` means only that virtual CPUs started. It
does not prove that the guest booted, mounted its environment or started the
keeper. Guest readiness must be an explicit protocol message.

### virtio-vsock

Firecracker maps guest AF_VSOCK ports to host Unix sockets. Proposed guest
ports:

```text
4999  bootstrap
5000  keeper control protocol
5001  embedded SSH
5002  health and lifecycle
6000  guest-to-host egress
```

The exact numbers are an internal protocol detail and should be versioned.

For a host-initiated connection, devcroft opens the Firecracker vsock backing
socket and sends:

```text
CONNECT 5000\n
```

Firecracker acknowledges the connection and turns the stream into a byte-for-
byte channel to the guest listener. This lets devcroft reuse the existing keeper
protocol for spawn, signal, resize, output and exit status.

The difference is transport only:

```text
process runtime:      UnixStream -> keeper
Firecracker runtime:  UnixStream -> Firecracker vsock -> keeper
```

A suitable seam is therefore a keeper transport abstraction, not a duplicate
session protocol.

## Keeper And SSH

The keeper should run inside the guest using the existing
`LocalSessionBackend`. It remains responsible for:

- session spawn and reaping;
- signal and PTY behavior;
- lifecycle hooks;
- service supervision;
- embedded SSH;
- policy application inside the guest.

The host preserves the existing local SSH socket contract:

```text
OpenSSH
  -> ProxyCommand devcroft proxy %n
  -> state/<name>/ssh.sock
  -> host vsock bridge
  -> guest vsock port 5001
  -> embedded russh server
```

Users and editors continue to see `<name>.devcroft`. Nothing needs to listen on
a host TCP port, and the current `ssh-config` format can remain unchanged.

The control socket follows the same shape. The host-side bridge connects a
local Unix socket to guest control port 5000.

## Guest Bootstrap

Firecracker requires a guest kernel and root filesystem with a real init
process. A minimal `/sbin/devcroft-init` should run as PID 1 and perform:

```text
Linux boot
  -> mount rootfs read-only where possible
  -> mount provider closure read-only
  -> mount workspace
  -> mount private state and HOME
  -> establish instance identity
  -> receive and verify launch specification
  -> apply guest nono policy
  -> run activation and lifecycle hooks
  -> start services and keeper listeners
  -> report READY over vsock
  -> reap descendants until shutdown
```

The host should not treat a running Firecracker PID as a healthy sandbox. A
usable instance must pass all three checks:

```text
VMM process identity is valid
Firecracker API socket responds
guest keeper answers a health request over vsock
```

## Configuration Delivery

Static, non-sensitive boot configuration may be delivered through a small
read-only config disk, initrd data or Firecracker's metadata service. Dynamic
or sensitive values should travel over an authenticated bootstrap channel on
vsock after boot.

The launch specification may contain:

```json
{
  "protocol": 1,
  "instance_id": "...",
  "runtime_fingerprint": "...",
  "policy_hash": "...",
  "workspace_mode": "clone",
  "control_port": 5000,
  "ssh_port": 5001
}
```

The bootstrap channel may additionally deliver:

- environment diff and unsets;
- compiled guest policy;
- SSH host key and authorized client key;
- egress proxy token;
- service plan;
- lifecycle hooks;
- fresh instance identity after snapshot restore.

Sensitive values must be removed from the init and keeper environments before
sessions can inherit them, following the same rule the process runtime applies
to SSH key material today.

The launch message should be authenticated and bound to the instance ID,
workspace disk identity, environment fingerprint, policy hash and snapshot
generation. This prevents one restored VM from accidentally accepting another
VM's launch state.

## Environment Closure In The Guest

The guest must not receive the host Nix daemon socket. Giving an agent package-
manager authority over a host-global store would violate the existing
provisioning boundary and create cross-agent authority.

The target layout is:

```text
shared immutable assets
  kernel
  minimal rootfs
  provider closure image

per-sandbox writable assets
  workspace disk
  HOME and cache disk
  service data
  logs and instance state
```

The provider closure should be materialized once on the host, converted to a
read-only filesystem image such as EROFS or SquashFS, and attached to every VM
using the same environment fingerprint.

```text
shared read-only closure image
          |
     +----+----+---------+
     v         v         v
   VM A      VM B      VM C

private disk A
private disk B
private disk C
```

Inside the guest, the closure is mounted at the expected store path. This
preserves devcroft's main density property: packages are materialized once and
shared read-only, even though each microVM has private memory and writable
storage.

The exact export mechanism must be measured. It must preserve symlinks,
permissions, executability and absolute store paths, and it must be fast enough
that image creation does not dominate `up`.

## Networking Without TAP

The recommended default is to attach no `virtio-net` device. Firecracker then
needs no TAP interface, NAT, forwarding rules, slirp process or host network
capability.

Guest egress uses vsock:

```text
application
  -> HTTP_PROXY on guest loopback
  -> guest relay
  -> AF_VSOCK CID 2, port 6000
  -> per-VM host Unix socket
  -> devcroft egress proxy
  -> allowed upstream
```

Properties:

- the guest has no direct IP route to the host or Internet;
- ignoring proxy variables does not produce an alternate route;
- UDP, QUIC and raw-socket egress have nowhere to go;
- every allowed request is attributable to one VM;
- the host proxy remains outside the guest policy domain;
- credential brokering can remain outside the VM;
- `/dev/net/tun`, `CAP_NET_ADMIN` and host firewall setup are unnecessary.

The existing per-sandbox proxy token should remain even though vsock already
separates instances. It is useful defense in depth and preserves the current
authentication contract.

Firecracker itself performs no egress filtering when a network device is
attached. If a future mode adds `virtio-net`, host firewalling becomes mandatory
and the vsock-only guarantee no longer applies. That must be a separately named
capability, not an invisible optimization.

## Service Access

Services can keep their declared in-guest ports. Every microVM has a private
port table, so all instances may run PostgreSQL on 5432.

SSH local forwarding works naturally because the embedded SSH server is already
inside the guest and can connect to guest loopback.

For an explicit host mapping:

```text
host 127.0.0.1:15432
  -> devcroft host listener
  -> vsock guest ingress port
  -> guest relay
  -> 127.0.0.1:5432
```

The host-side endpoint is allocated per sandbox and shown in `status`. The
service's own command and port remain unchanged.

## Workspace Modes

### `microvm-clone`

This is the first and strongest Firecracker deliverable.

```text
host bare mirror or source repository
           |
           | clone / bundle / seed image
           v
private guest workspace disk
           |
           v
agent edits, builds and commits
           |
           | commit / patch / bundle export
           v
host-side review and integration
```

Properties:

- the guest never sees the original host checkout;
- a destructive command cannot modify the original worktree;
- each agent has private refs, index, build outputs and service data;
- rollback can discard or restore the private disk;
- results leave through an explicit commit, patch or Git bundle;
- this mode can support a stronger hostile-code threat model once the host
  launcher and artifact chain are hardened.

The integration operation must remain explicit. Agents should not push directly
to upstream repositories from inside the guest by default.

### `microvm-shared`

This mode exposes a host worktree directly to the guest through a filesystem
server, likely virtio-fs through a vhost-user device.

```text
guest /workspace
  -> virtio-fs
  -> restricted virtiofsd
  -> one host worktree
```

Properties:

- editor and host see changes immediately;
- no patch/export step is required;
- the agent can modify or delete the real worktree;
- the filesystem server becomes part of the host attack surface;
- performance and filesystem semantics differ from native access;
- snapshot compatibility depends on Firecracker's device support.

As of this exploration, Firecracker's generic vhost-user device supporting a
`virtiofsd` backend is still draft/open work, and snapshotting with such a
device is refused. The shared mode should therefore not block clone mode and
must not be built on an unstable interface without a deliberate maintenance
commitment.

devcroft should not implement a new filesystem protocol or adopt 9p merely to
ship this mode sooner.

## Host Launcher And Jailer

Firecracker recommends its jailer, or constraints at least as restrictive, for
production isolation. The jailer:

- constructs a mount namespace and chroot;
- creates cgroups and applies resource controls;
- creates and grants `/dev/kvm` and `/dev/net/tun` inside the jail;
- changes uid and gid;
- optionally creates a PID namespace;
- clears the environment;
- copies the matching Firecracker binary into the jail;
- treats kernel, rootfs, disk and namespace paths as trusted inputs.

The official jailer is designed to run with privileges and recommends a unique
uid/gid per VM for stronger isolation.

### Local development profile

A first local implementation may run without the jailer if:

- the user already has read/write permission on `/dev/kvm`;
- no `virtio-net` or TAP setup is needed;
- Firecracker's own seccomp filters remain enabled;
- the VMM is placed in a delegated cgroup;
- nono restricts the final VMM process to its images, devices and sockets;
- launch artifacts are verified and stored outside project-controlled paths;
- failure of any required constraint refuses the Firecracker mode.

This can legitimately be called microVM hardening. It should not be marketed as
equivalent to Firecracker's recommended production multi-tenant setup.

### Strong boundary profile

A stronger claim requires:

- the official jailer or a measured equivalent;
- a dedicated uid/gid and private jail per VM;
- mandatory cgroup v2 resource limits;
- root-owned or otherwise immutable trusted launch artifacts;
- current host kernel, microcode and virtualization mitigations;
- no unfiltered network interface;
- bounded logs and disk quotas;
- watchdog and unresponsive-VMM handling;
- authenticated snapshot and disk lifecycle;
- no fallback to an unconstrained launcher.

### Ordering with Landlock

The gVisor result remains relevant: Landlock cannot be applied before a launcher
that still needs `mount()`, `pivot_root()` or jail construction.

Incorrect:

```text
apply nono
  -> start jailer
       -> mount/pivot_root fails
```

Correct:

```text
jailer constructs namespaces and chroot
  -> starts final Firecracker process
       -> apply any compatible final-process restriction
```

If the official jailer leaves no hook for applying nono after its setup, its
own namespace, chroot, cgroup, uid/gid and seccomp constraints remain the
host-side enforcement. Nono still runs inside the guest.

## Lifecycle

A graceful shutdown sequence should be:

```text
devcroft down
  -> send guest SHUTDOWN over control vsock
  -> keeper terminates sessions and services
  -> guest init reports QUIESCED
  -> guest powers off
  -> wait for Firecracker process
  -> SIGTERM on timeout
  -> cgroup.kill / SIGKILL as final escalation
  -> remove sockets and instance state
```

Host metadata must record enough identity to reject PID reuse and stale state:

```json
{
  "runtime": "firecracker",
  "instance_id": "...",
  "firecracker_pid": 1234,
  "api_socket": "...",
  "vsock_socket": "...",
  "guest_cid": 42,
  "kernel_digest": "...",
  "rootfs_digest": "...",
  "runtime_fingerprint": "...",
  "workspace_disk": "...",
  "snapshot_generation": "..."
}
```

`down`, `rm`, recovery and adoption must validate process identity rather than
trusting a PID file alone.

## Logs And Metrics

Keep sources separate on disk:

```text
logs/
  vmm.log
  vmm.metrics
  guest-boot.log
  keeper.log
  services.log
  egress.log
```

`devcroft logs` may combine them for display using source prefixes. Log and
serial-console storage must be bounded because a guest can influence their
volume. Unbounded logs turn guest behavior into a host disk-exhaustion path.

Metrics should distinguish:

- Firecracker/VMM health;
- guest boot and keeper health;
- cgroup CPU, memory and PID events;
- block and vsock rate-limit events;
- service state;
- egress decisions.

## Snapshots And Warm Pools

Firecracker snapshots include guest memory and device state. Disk files remain
the integrator's responsibility. Restore can demand-page a shared memory image,
but snapshot state is tied to compatible Firecracker and hardware
configurations.

A reusable fleet snapshot must be created before instance-specific state is
introduced:

```text
boot minimal guest
  -> mount immutable base runtime
  -> start devcroft-init
  -> wait at bootstrap boundary
  -> create base snapshot

restore N instances
  -> assign fresh instance ID and vsock path
  -> attach private workspace and state disks
  -> inject fresh policy and proxy token
  -> start keeper and services
```

The base snapshot must contain no:

- user credentials;
- proxy tokens;
- SSH private identity intended to be unique;
- project workspace;
- authenticated agent state;
- reusable instance IDs.

Firecracker updates VMGenID and can reseed the guest kernel on supported guest
kernels, but it does not repair IDs, cached randomness or tokens already
generated by userspace. Snapshot cloning must regenerate all such state.

Open vsock connections are reset by snapshot creation/restore. Host bridges must
be re-established with a new backing socket path before the guest is declared
ready.

Snapshots and memory files are trusted Firecracker inputs. Devcroft must protect
their integrity, ownership and lifecycle; Firecracker performs only limited
sanity checking and does not authenticate the complete snapshot set.

## Suggested Architecture Seams

Firecracker is larger than the current `SessionBackend` abstraction. It changes
the placement of the keeper, workspace, network and environment.

The new top-level seam should represent the runtime:

```rust
trait SandboxRuntime {
    fn prepare(&self, request: &PrepareRequest) -> Result<PreparedRuntime>;
    fn start(&self, prepared: PreparedRuntime) -> Result<RunningRuntime>;
    fn connect_control(&self, running: &RunningRuntime) -> Result<RuntimeStream>;
    fn connect_ssh(&self, running: &RunningRuntime) -> Result<RuntimeStream>;
    fn inspect(&self, running: &RunningRuntime) -> Result<RuntimeStatus>;
    fn stop(&self, running: RunningRuntime) -> Result<()>;
}
```

Conceptual structure:

```text
SandboxRuntime
+-- ProcessRuntime
|   +-- restricted local keeper
|   +-- direct Unix transport
|
+-- FirecrackerRuntime
    +-- VmmProcess
    |   +-- API client
    |   +-- PID/cgroup identity
    |   +-- logs and metrics
    +-- GuestTransport
    |   +-- bootstrap vsock
    |   +-- control vsock
    |   +-- SSH vsock
    |   +-- egress vsock
    +-- DiskSet
    |   +-- rootfs
    |   +-- provider closure
    |   +-- workspace
    |   +-- private state
    +-- SnapshotManager
```

The existing `SessionBackend` remains useful inside each keeper. It should not
be stretched into owning VMM setup, disks or guest boot.

The Firecracker API client must remain internal to `FirecrackerRuntime`.
Generic lifecycle, keeper and SSH code should depend on runtime operations, not
Firecracker endpoints.

## Phased Validation Plan

### Phase 0: isolated spike

No public CLI or manifest change.

1. Probe `/dev/kvm`, host architecture and Firecracker version.
2. Boot a verified kernel and immutable minimal rootfs without `virtio-net`.
3. Start a minimal `devcroft-init` as guest PID 1.
4. Prove host-to-guest and guest-to-host vsock transport.
5. Run one command and propagate stdout, stderr and exit status.
6. Prove direct TCP, UDP and host-loopback access are absent.
7. Route one allowlisted HTTP request through the existing egress proxy over
   vsock and refuse another.
8. Restrict the VMM with nono and confirm the VM still boots.
9. Measure cold boot, ready time, RSS and disk cost.
10. Restore a pre-identity snapshot and inject fresh instance state.

Gate: do not start CLI integration if the guest reaches an undeclared host
path, network route, control socket or another VM's state.

### Phase 1: `microvm-clone`

1. Introduce `SandboxRuntime` and `ProcessRuntime` without behavior change.
2. Add the typed Firecracker API client.
3. Define kernel/rootfs provenance and update policy.
4. Build guest init and guest keeper packaging.
5. Export provider closures as shared read-only images.
6. Create private workspace and state disks.
7. Transport control and SSH over vsock.
8. Integrate vsock-only egress and service forwarding.
9. Implement Git clone, commit, patch and bundle exchange.
10. Add cgroup limits, watchdog and complete cleanup.
11. Add a pre-identity warm snapshot.
12. Run real Rust, Go, Node, JVM, PostgreSQL and process-compose workloads.

### Phase 2: strong launcher

1. Decide official jailer versus a measured equivalent.
2. Add per-VM uid/gid and private jail ownership.
3. Make cgroup and host security preflights mandatory.
4. Verify all kernel, rootfs, closure and snapshot artifacts.
5. Bound logs, disks, memory, CPU and PIDs.
6. Implement crash recovery and stale VM adoption.
7. Publish a Firecracker-specific threat model.
8. Remove every fallback that would silently weaken the requested runtime.

### Phase 3: `microvm-shared`

1. Require a stable Firecracker generic vhost-user/virtio-fs interface.
2. Run `virtiofsd` as a separate restricted host process.
3. Give it a mount view and nono policy containing exactly one worktree.
4. Test symlinks, hardlinks, rename, locks, mmap and teardown.
5. Measure representative compiler and package-manager performance.
6. Report the host-worktree write exposure explicitly.
7. Refuse snapshots while the device is not snapshot-compatible.

## Acceptance Criteria Before Product Claims

Firecracker becomes a devcroft feature only after the implementation proves:

1. Cold boot and warm restore on ordinary supported hardware.
2. Real Rust, Go, Node and JVM toolchains in the guest.
3. A provider closure shared read-only without exposing the Nix daemon.
4. Keeper control, SSH, SFTP and port forwarding over vsock.
5. Egress possible only through the authenticated host proxy by default.
6. Complete `down` and `rm` cleanup of VMMs, cgroups, sockets and disks.
7. A private workspace flow with reliable commit/patch export.
8. Enforced CPU, memory, PID, disk and log limits.
9. Verified kernel/rootfs provenance and an upgrade policy.
10. A host launcher with jailer-equivalent guarantees for the strong profile.
11. Snapshot cloning without duplicated identities, credentials or userspace
    random state.
12. Separate, testable threat-model statements for clone and shared modes.

## Risks

### Scope expansion

This runtime adds VMM lifecycle, guest images, guest init, disk management,
snapshot management and host/guest transport. It can overwhelm the project if
devcroft also attempts to replace nono, Nix, Git or the service supervisor at
the same time.

The intended ownership remains:

| Component | Owner |
|---|---|
| Environment materialization | flox, Nix, devbox, devenv |
| Least-privilege policy | nono |
| VM isolation | Firecracker/KVM |
| VM jail and resources | Firecracker jailer plus cgroups |
| Lifecycle, workspace, services, SSH composition | devcroft |
| Agent task scheduling | outside this runtime change |

### External binary requirement

Firecracker is a VMM binary controlled through an API socket. A Rust client
crate does not turn it into an embedded VMM. Adopting it explicitly reverses
devcroft's current rule against an external sandboxing binary for the keeper
path.

That reversal can be justified only as a named optional runtime with a stronger
boundary, not hidden as an implementation detail.

### Artifact supply chain

The kernel, rootfs, Firecracker binary, jailer, closure image and snapshots all
become trusted inputs. They require hashes, provenance, ownership rules and an
upgrade process. Project-controlled files must never select arbitrary host paths
for these artifacts.

### Platform split

Firecracker is Linux/KVM. It does not replace macOS process mode or the proposed
Swift provider. A unified product therefore has two runtime capability matrices,
not one cross-platform guarantee.

### Workspace ambiguity

`clone` and `shared` do not provide the same protection. If both are displayed
as merely "Firecracker enabled," users will reasonably assume the original
worktree is protected in both. The workspace mode must remain visible in
configuration, status and policy rendering.

## Recommendation

Proceed only as a new OpenSpec change, tentatively `add-firecracker-runtime`,
with these initial constraints:

- Linux-only;
- explicit opt-in;
- `microvm-clone` first;
- no `virtio-net` by default;
- vsock for control, SSH, egress and service forwarding;
- provider closure exported once and attached read-only;
- no Nix daemon socket in the guest;
- keeper and nono policy inside the guest;
- nono or jailer restrictions around host components;
- no claim of production-grade isolation before the strong-launcher gates pass;
- `microvm-shared` specified and released separately.

If those constraints hold, Firecracker could extend devcroft from a native
accident-containment tool into one control plane spanning fast local development
and stronger per-agent microVM isolation without changing the project's
environment, editor or day-to-day command surface.

---

## Review (devcroft, 2026-09-15)

Read against the repository as of `dev` at `c56ff80`: the shipped invariants
(`CLAUDE.md`), the two changes this overlaps (`add-linux-agent-fleet`,
`add-macos-service-vm`), the proposal it should pair with
(`add-devcontainer-provider`), and `docs/threat-model.md`. Technically the
exploration is right almost everywhere; what it lacks is position relative to
what already exists. That changes the recommendation more than any technical
point would.

### What it gets right

- **The gVisor lesson, applied rather than repeated.** gVisor sat in the
  squeezed middle; Firecracker is the strong end. "Jailer first, then any
  restriction on the final process" is `remove-gvisor-backend`'s measurement
  — Landlock cannot precede a launcher that needs `mount()` — stated
  correctly.
- **`runtime` × `workspace` as separate axes.** This is the document's real
  contribution. A single `hardened` flag would hide the difference between
  "the guest sees your worktree" and "the guest has a clone"; `clone` first
  and `shared` later, with the reason (Firecracker's vhost-user/virtio-fs
  is still open work, and snapshots with such a device are refused), is
  the right order and the right caution.
- **No `virtio-net`; egress over vsock only.** It reproduces the shipped
  topology — loopback-only namespace, unix-socket relay to the host proxy —
  one layer down, and keeps the proxy and its per-sandbox token. This is
  what makes the design *devcroft's* rather than "Firecracker with a CLI".
- **nono in the guest.** Without a guest policy an agent isolated from the
  host still has everything in the guest. And keeping the nix daemon out
  of the guest is exactly `sandbox-provisioning`'s line.
- **Honesty about the invariants it reverses.** It says itself that it
  reverses "the keeper SHALL NOT be executed as a child of a separate
  sandboxing binary" and "the process tier requires no external backend
  binary", and asks that the reversal be named, not hidden. That is this
  project's voice.

### What it does not position itself against

1. **The two changes it overlaps.** `add-linux-agent-fleet` already has a
   `devcroft-init` as PID 1, and lists cgroups and a PID namespace as its
   remaining work; `add-macos-service-vm` already has a Linux guest with an
   undefined host/guest protocol (`docs/spec-review-2026-09-14.md`, #2, #3,
   #5). A Firecracker runtime *solves* fleet's isolation half by
   construction — one kernel per agent is PID isolation and a resource
   budget for free — and *is* the service VM's guest. As a third parallel
   design it would triple the review's findings; as the successor to both
   halves it is a simplification. The document has to say which it is.

2. **The alternative devcroft already supports: devcroft, unchanged, inside
   a VM.** `docs/threat-model.md` names it — "for a real boundary, run
   devcroft in a VM; that is already how the macOS path works". A Lima or
   OrbStack VM with devcroft and N process sandboxes in it gives the
   boundary against the host today, with no code. What it does not give is
   a boundary *between* agents, or a budget per agent. So the document's
   differentiator is not "a VM"; it is **one VM per agent, with the closure
   shared read-only across VMs**. That should be the Summary's first
   sentence, and the first thing measured: N microVMs versus one VM with N
   sandboxes, in boot, RSS and disk, on ordinary hardware.

3. **The strongest pairing, missed: the devcontainer provider.** An image's
   rootfs *is* a guest disk. For Nix the closure has to be exported as
   EROFS/SquashFS and mounted at `/nix/store` — real work, to be measured.
   For the image `devcontainer.json` names, the guest simply is the image.
   And `add-devcontainer-provider`'s D3 — the two-level view, because the
   host-built keeper cannot run on the image's libc — disappears in a
   guest: the keeper runs on the image's libc there, full stop. If a
   microVM runtime is ever built, the `image` tier is its natural first
   consumer, not Nix.

4. **macOS.** Correct that Firecracker is Linux/KVM. But
   `docs/prior-art.md` already records `apple/containerization`
   (Virtualization.framework microVMs). `SandboxRuntime` should be a
   *microVM* runtime with two possible backends, or macOS gets a fourth
   separate design for the same idea.

5. **The threat model.** `clone` mode targets use case B in
   `docs/threat-model.md` — "unreviewed code, many instances: not served,
   must not be claimed". This is the first proposal under which B becomes
   servable. That is the *title* of the threat-model revision, not a
   sentence under Risks.

6. **Minor, technical.** `InstanceStart` ≠ guest ready — well caught.
   Hardcoded vsock ports — "versioned" is the right answer. The closure
   export must preserve, besides symlinks, permissions and absolute store
   paths, the store's **mtime-of-1 and hardlinks**, or the same fingerprint
   yields a different image on two hosts.

### Recommendation

Yes to `/opsx:explore`, not directly to a proposal, and with three conditions
the document does not carry:

- **It declares itself the successor** to `add-linux-agent-fleet`'s
  isolation half and `add-macos-service-vm`'s guest; both are amended or
  superseded, not left standing beside it.
- **Phase 0 is smaller than the document's.** Before any code: devcroft *as
  it is*, run by hand in N Firecracker microVMs with one closure exported
  once — a script, not a runtime — to measure whether density and boot on
  ordinary hardware support "one VM per agent" at all. If the numbers do
  not, nothing after (API client, init, snapshots, bundle export) is worth
  writing.
- **The first consumer is the devcontainer provider, not Nix**: guest =
  image, no EROFS export, no nix daemon to keep out. Nix follows if the
  export measures acceptably.

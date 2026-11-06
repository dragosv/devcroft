## Context

`Provider::resolve` and tooling resolution currently execute on the host before
the keeper starts. Hook provisioning already has a separate restricted process.
`SessionBackend` returns local process handles; SSH SFTP and forwarding use the
server's filesystem and network. These are useful components, not an existing
VM runtime. The new path is conditional on the accepted microVM experiment.

## Goals / Non-Goals

**Goals:** a Linux guest execution contract for local and fleet use, confinement
before all project execution, inspectable effective authority, durable private
state and complete guest session semantics.

**Non-Goals:** protect manual host cloning retroactively, run Darwin inside Linux,
prove absence of VMM escapes, hide a directly injected credential from its guest,
ship a broker or arbitrary IP networking, or choose the winning VMM here.

## Decisions

### R1 — Separate host control from guest execution

One helper embeds the selected VMM for one guest. Controller, proxy and guest
keeper are separate roles. Keep `LocalSessionBackend` in the guest, where its
PID/PGID/PTY operations are correct. Host exec/proxy clients connect through a
bounded transport adapter. Guest PIDs never become host `kill()` arguments.

SSH terminates in the guest, so SFTP and `direct-tcpip` refer to guest resources.
Host endpoints are private local sockets, not implicit public TCP listeners.
Each instance has distinct keys and generation-bound endpoints; shared images
contain no SSH host key, client secret or session credential.

### R2 — Bootstrap is trusted independently of the repository

The controller selects versioned helper/kernel/init/runtime artifacts from a
user-trusted distribution manifest, verifies hashes against that authority and
checks compatibility before launch. A project's self-supplied hash is not
authorization to replace bootstrap. Image providers supply payload filesystems,
not arbitrary host executables or launch profiles.

The handshake binds instance generation, protocol version, target, boot artifact
identity and required capabilities. The controller verifies its own launch
inventory; a guest's claim to be confined is not authoritative. Unknown messages,
oversized frames, timeouts and mismatches fail closed. Transport cannot request
arbitrary host paths, commands, sockets or another instance's state.

### R3 — Materialization is inside the boundary too

Start a private Linux preparation VM before evaluating provider definitions,
building packages, extracting untrusted OCI layers or executing lifecycle code.
No user-installed Nix/flox/devbox/Docker/toolchain is required on the host.
Host parsing is bounded data parsing only; it does not source files or evaluate
project expressions. Include tooling-layer resolution in this rule.

Initially use disposable preparation jobs with private writable package stores.
Share only immutable verified inputs. Reject a single mutable builder shared by
untrusted projects: it would reintroduce cross-project authority behind one VM.
Reusing a clean boot artifact is fine; reusing unreviewed writable builder state
across projects is not. A local adaptable instance may do private administrative
setup, but it cannot publish itself into the shared cache.

Capture env/unset, hook data, services, closure roots and provider-generated
profiles for the **Linux** target. Use the same guest project path in preparation
and runtime instead of rewriting arbitrary path strings. Darwin resolutions and
host-linked Swift/Xcode artifacts are not accepted by this path.

Publication is a host-owned transaction: stop writers, validate a bounded output
manifest, hash immutable bytes, assign a content identity, then atomically
publish and lease the object. Do not mount a guest filesystem on the host merely
to inspect it. A private guest cannot choose an existing cache object's path or
claim another recipe's provenance. Content hashing proves bytes, not goodness;
keep recipe identity and verification evidence separately. Failed or interrupted
publication remains quarantined and does not replace a valid entry.

### R4 — Project declarations request authority

Compute effective authority from project requests intersected with user-owned
grants and runtime hard limits. Bind user decisions to project identity, selected
definition and phase. Imported external definitions are requests too. Neither
`[env] forward` nor global credential settings automatically authorize secrets
for an unfamiliar VM project; resolve references only under explicit user scope.
Existing user grants can be reused when their bound scope still matches.

Initial private mode provides no host filesystem mounts, daemon sockets, SSH
agent or generic device passthrough. Setup may administer its private guest;
runtime receives its declared guest policy. Required unsupported requests are
named and refused, not silently ignored. Render the combined plan with provenance:
host file/device descriptors and modes, transport direction and endpoints,
guest grants, phase-specific egress, credential reference names and resource
limits. Never render secret values.

### R5 — Enforce egress outside the guest and revoke setup authority

Initial topology: guest loopback plus explicit vsock channels to a host proxy;
no NIC, TSI, NAT or generic host socket relay. Authenticate per instance and phase.
The host proxy enforces destination policy and refuses unintended host-local,
link-local and private destinations unless explicitly granted; account for DNS
changes, redirect targets and IPv6. HTTP/CONNECT support is not arbitrary UDP.

At setup completion, terminate its process tree, revoke its token and close its
existing proxy streams before runtime admission. Merely changing environment
variables does not revoke connected sockets. If cleanup fails, runtime does not
start. Disable unimplemented reverse forwarding, agent forwarding and X11;
forwarded guest ports cannot become generic host-connect operations.

Direct credentials, if explicitly authorized, arrive after boot over the scoped
channel, never in kernel args, shared images, controller state or logs. A guest
can still persist a secret it receives. For that reason ordinary local disks are
never published as shared fleet bases. The first shared-artifact preparation
path injects no personal credentials; credential-dependent publication needs a
separate design rather than a false promise of reliable secret scrubbing.

### R6 — Persistent state and cleanup have different owners

Host state records runtime kind/version, target, immutable artifact identities,
private disk identity, instance generation and a host process identity stronger
than PID alone. Guest, transport, keeper, environment freshness and service
readiness are separate states. A live helper does not imply a usable project.

`down` stops services, requests guest sync/shutdown and retains private data.
Timeout escalates to forced termination with an explicit crash result. Restart
checks compatibility and disk recovery; incompatible state is refused without
deletion. `rm` removes only the selected instance's owned resources after stop;
shared artifacts use leases/reference tracking and separate garbage collection.
Deleting a controller record must not silently orphan an active VMM.

Set finite guest memory/vCPU, disk, log and transport budgets. Account for helper
and proxy overhead separately. Required host resource controls must be qualified
per platform; configurable guest RAM is not a total host-memory cap. Do not ship
an untrusted-project claim on a row whose required controls are unavailable.

## Risks / Trade-offs

- Disposable builders cost more → measure cold preparation and immutable reuse;
  defer mutable sharing until its authority is separately proven.
- Host/controller parsers remain attack surfaces → bounded formats, negative
  tests and no host filesystem mounting of guest-controlled images.
- Guest root defeats guest Landlock → host RO disks, channel controls and helper
  confinement remain mandatory and independently tested.
- SSH clients/editors parse guest data → keep their exposure explicit in the
  threat model; remote editing does not prove the host editor immune to bugs.
- Stronger guarantees exclude some networking/setup workflows → capability
  refusals identify the missing feature and phase, without fallback.

## Migration Plan

Implement as opt-in VM dispatch while experiments qualify platforms. Preserve
current state schema decoding and native instance behavior. The runtime kind is
immutable for an existing instance: migration creates a new instance with explicit
import, never reinterprets a native directory as a VM disk.

Before making VM the default for the proposed Linux-project 1.0 workflow, update
`docs/decisions.md`, `docs/threat-model.md`, README, roadmap, CLI help and
`openspec/config.yaml` together; reconcile provider/lifecycle/policy/SSH deltas
and the current fleet design. Label the explicit native/Darwin route accurately.
Rollback disables creation of new VM instances but preserves disks and allows
supported cleanup/export; it never downgrades an existing VM into host execution.

## Open Questions

The experiment resolves VMM choice and exact confinement/transport mechanics.
Distribution trust roots, update/rollback compatibility windows and release OS
matrix must be settled before packaging. Default-switch timing and CLI spelling
are migration decisions, not permission to weaken the contract.

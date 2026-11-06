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
make a host editor safe against a hostile guest, ship a broker or arbitrary IP
networking, or choose the winning VMM here.

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

A guest server cannot enforce what a host client offers, since guest root can
replace it. Agent, X11 and remote forwarding are therefore disabled in the client
configuration devcroft generates and in the options `devcroft ssh` passes. The
server's refusal is a convenience, not the control.

### R2 — Bootstrap is trusted independently of the repository

The controller selects versioned helper/kernel/init/runtime artifacts, and the
preparation image holding the pinned provider tools, from a user-trusted
distribution manifest, verifies hashes against that authority and checks
compatibility before launch. A project's self-supplied hash is not
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

A preparation job receives a declared input set, not the project. The controller
attaches only those files and hashes them into the recipe identity, so a file
outside the set cannot change the artifact. A provider that cannot enumerate its
inputs, a flake importing `./nix/shell.nix` being the ordinary case, uses the
whole selected source as its input set, and its artifact is reusable only for
that source. `manifest_fingerprint` is a staleness hint, not an artifact identity.

The cache is namespaced by project ID. An output from one project's preparation
is never served to another, because it is bytes a hostile job chose and the
consuming guest's kernel parses them as a filesystem. Reuse across projects is
limited to artifacts fetched as fixed-output or verified against a signature the
user trusts. Within a project a namespace does not make two sources equal: the
input-set rule above keeps one branch's job out of another's artifact.

Capture env/unset, hook data, services, closure roots and provider-generated
profiles for the **Linux** target. Use the same guest project path in preparation
and runtime instead of rewriting arbitrary path strings. Darwin resolutions and
host-linked Swift/Xcode artifacts are not accepted by this path.

Every provider requalifies for this target. A lock that does not cover the Linux
target is refused by name, never silently re-locked. Preparation holds no personal
credentials, so a path that needs them is refused up front; resolving an unlocked
flox manifest, which requires a FloxHub login, is the known case.

Publication is a host-owned transaction: stop writers, validate a bounded output
manifest, hash immutable bytes, assign a content identity, then atomically
publish and lease the object. Manifest and bytes are guest-produced, so validation
covers size, format envelope and identity assignment, not what the bytes mean. Do
not mount a guest filesystem on the host merely to inspect it. A private guest
cannot choose an existing cache object's path or claim another recipe's provenance. Content hashing proves bytes, not goodness;
keep recipe identity and verification evidence separately. Failed or interrupted
publication remains quarantined and does not replace a valid entry.

### R4 — Project declarations request authority

Compute effective authority from project requests intersected with user-owned
grants and runtime hard limits. Bind user decisions to project identity, selected
definition and phase. Imported external definitions are requests too. Neither
`[env] forward` nor global credential settings automatically authorize secrets
for an unfamiliar VM project; resolve references only under explicit user scope.
Existing user grants can be reused when their bound scope still matches, with one
exception: direct credentials are stated per instance. A new instance never
inherits them because its project identity matches, and a refresh that brings in
another source drops them until they are restated. An external PR fetched into a
trusted checkout has the same project identity and definition, and origin comes
from project-controlled `.git/config`, so neither can be what authorizes a key.

The intersection needs a value on first use, when the user has granted nothing.
Devcroft ships a versioned, user-replaceable exploration profile: preparation and
setup may reach the package registries of the selected source and base, runtime
gets nothing from it, and it carries no credentials. It renders like any grant,
with origin `profile:exploration`. Those hosts accept uploads, so the profile is a
disclosed outbound path, not a safe default. The proxy reports refused
destinations so a local session can turn them into declared requests. The host
list belongs to the base catalog.

Initial private mode provides no host filesystem mounts, daemon sockets, SSH
agent or generic device passthrough. Setup may administer its private guest;
runtime receives its declared guest policy. Required unsupported requests are
named and refused, not silently ignored. Render the combined plan with provenance:
host file/device descriptors and modes, transport direction and endpoints,
guest grants, phase-specific egress, credential reference names and resource
limits. Never render secret values.

### R5 — Enforce egress outside the guest and revoke setup authority

Initial topology: guest loopback plus explicit vsock channels to a host proxy;
no NIC, TSI, NAT or generic host socket relay. Instance identity comes from the
channel the controller created for that instance: nothing the guest sends can name
another, and a token held in the guest is readable by guest root, so it is not
identity. Phase is proxy state the host keeps for that channel.
The host proxy enforces destination policy and refuses unintended host-local,
link-local and private destinations unless explicitly granted; account for DNS
changes, redirect targets and IPv6. `nono::HostFilter` covers link-local and
metadata hosts only, so the rest is new code. HTTP/CONNECT support is not
arbitrary UDP. Forwarded guest ports cannot become generic host-connect operations.

Setup ends on the host: switch the channel to runtime policy and close streams
opened under setup. The guest cannot refuse this, and if it cannot be completed
runtime does not start. Merely changing environment variables does not revoke
connected sockets. What the host cannot do is confirm that setup's processes are
gone, since it would be asking a guest that may have run setup as root. So the
guest is one trust domain once setup has had root: a surviving process, or
anything setup wrote to the writable layer, continues into runtime, and runtime
guest policy is not a boundary against it. Creation-time setup therefore ends with
a guest reboot, which ends processes and leaves persisted changes in place.

Local adaptation has to return to setup after runtime has begun. A setup window
is a host operation on a running instance: it moves the channel to setup policy
and opens an administrative session served by a guest service that init starts
before the keeper restricts itself, since keeper sessions can never regain
authority. That session is reached over the control channel, not the project's SSH
endpoint. A window widens egress for the whole guest rather than one session, ends
on request or timeout and shows in status. Opening one on an instance holding
direct credentials names that exposure first.

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

The controller is a role, not a daemon. Each instance has one long-lived helper
and one host proxy with tied lifetimes, and the CLI acts as controller under a
per-instance state lock. Leases are controller-state records carrying instance
generation, so a crash leaves them recoverable. Errors keep the existing layers
and exit codes: `provider` for preparation, `backend` for helper, VMM and host
confinement, `keeper` for the guest keeper and its transport, `ssh` for guest SSH.

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

### R7 — Runtime kind is user-owned

Repository content never selects or implies a runtime. A manifest, a detected
source or a Darwin-only provider such as `swift` is a request, and a request for
native execution needs a user-owned grant bound to the project identity. Without
one, `up` uses the VM path where a row is qualified and otherwise refuses, naming
the grant. It never prompts without a terminal. Native stays available, and
required for Darwin work, as a decision the user made for that project. Status
says which runtime is running and who chose it.

The rule ships with the first release that makes an untrusted-project claim: no
such claim can exist while plain `up` reaches host-side provider resolution for a
project nobody has reviewed. Until then VM stays opt-in and the existing contract
holds. Instances that already exist keep running as they are.

### R8 — Editors are classified by what a guest can do to them

Microsoft's Remote-SSH notice warns that a compromised remote can execute code on
the local machine. Calif demonstrated this on default settings in VS Code and
Cursor, and reports that Microsoft does not plan to change it. This is that
client's design rather than a bug to wait out, and a private workspace would make
it the main interface, so a hostile guest and such a client cannot share a claim.

The untrusted-project claim covers a terminal SSH client, SFTP transfers and a
guest-served editor reached through a local forward, with the browser as the
client. Remote-SSH and editors derived from it are for projects the user has
marked trusted, through a user-owned grant bound to project identity. `ssh-config`
emits no entry for an unmarked project, so an editor does not list it. That is a
default, not a control, since any SSH client can still be pointed at the proxy
command: the claim does not extend to that connection, and the threat model says
so. JetBrains Gateway and Zed stay unclassified until examined the same way.

### R9 — Distribution obligations belong to this change

Guest artifacts are distributed binaries. A Linux kernel and base images carry GPL
and other source obligations that `scripts/gen-third-party-licenses.py` does not
cover, since it reads Cargo dependencies. The macOS helper needs the Hypervisor
entitlement and signing, which `cargo install` does not provide. crates.io refuses
Git dependencies, so a VMM pinned to a commit cannot be in the published crate.
Packaging therefore decides a helper and artifact channel separate from the
crate, with its own licence inventory, before any VM release.

## Risks / Trade-offs

- Disposable builders cost more → measure cold preparation and immutable reuse;
  defer mutable sharing until its authority is separately proven.
- Host/controller parsers remain attack surfaces → bounded formats, negative
  tests and no host filesystem mounting of guest-controlled images.
- Guest root defeats guest Landlock → host RO disks, channel controls and helper
  confinement remain mandatory and independently tested.
- SSH clients and editors parse guest data → R8 keeps Remote-SSH outside the
  untrusted claim. A terminal still renders guest-controlled escape sequences and
  a guest-served editor shares `localhost` with other forwarded ports, so both
  stay named in the threat model.
- A one-way setup boundary cannot serve interactive adaptation → setup windows
  accept one trust domain in the guest rather than claim a separation guest root
  would void.
- Per-project caches give up reuse between projects → cost measured in
  `evaluate-microvm-runtimes`; widening it needs its own authority argument.
- Stronger guarantees exclude some networking/setup workflows → capability
  refusals identify the missing feature and phase, without fallback.

## Migration Plan

Implement as opt-in VM dispatch while experiments qualify platforms. Preserve
current state schema decoding and native instance behavior. The runtime kind is
immutable for an existing instance: migration creates a new instance with explicit
import, never reinterprets a native directory as a VM disk.

Before making VM the default for the proposed Linux-project 1.0 workflow, update
`docs/decisions.md`, `docs/threat-model.md`, README, roadmap, CLI help, `CLAUDE.md`
and `openspec/config.yaml` together; reconcile provider/lifecycle/policy/SSH deltas
and the current fleet design. That review names each invariant it changes: the
two-phase model, one isolation tier, environment resolving once at `up`, the
policy inputs, and the keeper-binary requirement, which now has to say what a
helper and a guest process are. It also reconciles `add-agent-interaction`, whose
approvals pass descriptors over `SCM_RIGHTS` and cannot cross a VM, and decides
what happens to the Lima stack in `add-macos-service-vm`. New commands join
`USAGE`. Label the explicit native/Darwin route accurately.
Rollback disables creation of new VM instances but preserves disks and allows
supported cleanup/export; it never downgrades an existing VM into host execution.

## Open Questions

The experiment resolves VMM choice and exact confinement/transport mechanics.
Distribution trust roots, update/rollback compatibility windows and release OS
matrix must be settled before packaging. Default-switch timing and CLI spelling
are migration decisions, not permission to weaken the contract. How an agent's
approval and attention requests reach the host needs a channel of its own: the
declared list has none yet, and none may be added outside the launch plan.

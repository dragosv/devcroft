## Context

The owner's 2026-11-10..12 documents, kept outside the repository, split a
service-backed VM sandbox into a **workload microVM** (`VM workload`: repository
code, builds, agents, keeper and SSH/editor sessions) and a **companion microVM**
(`VM companion`: stateful services from a trusted catalog). Adversarial review
checked those documents and two drafts of this change against the codebase, the
measured fixtures under `experiments/microvm/` and the existing VM changes.

On 2026-11-13 the owner recorded four decisions that govern this design:

1. **Asterinas is deferred.** Stage 1 runs Linux LTS in both VMs; the GOAL
   preference for Asterinas in the workload kernel stands, and
   `qualify-asterinas-companion-kernel` does not block this change.
2. **The companion guest base is chosen by measurement.** NixOS minimal (the
   choice stated in `INVESTIGATIE-NIX-ASTERINAS-VM-WORKLOAD-2026-11-11.md`) and
   a pinned closure + dedicated init are measured on the same fixture (task 1.4).
3. **Host channels to the companion are limited to `SHUTDOWN` and diagnostics,
   both host-initiated.** They are an approved exception to the 2026-11-11 rule
   ("no controller API, no direct host collector"). Readiness is probed by the
   host broker, and no per-instance secret is delivered to the companion.
4. **A crashed companion restarts automatically**, always through disk check and
   service recovery (C13).

Measured facts this design rests on:

- The durable recovery that `evaluate-microvm-runtimes` 3.7.1 measured used
  Redis with `appendonly yes` and `appendfsync always`
  (`experiments/microvm/workload_smoke.py:56-57`) and full flush semantics on
  every disk (`experiments/microvm/libkrun/src/main.rs:154-168`).
- At the libkrun pin, a guest kernel panic and a clean poweroff both end the
  helper with exit status 0. The evaluation therefore requires correlated
  controller intent and acknowledgement before an orderly stop is classified
  (`evaluate-microvm-runtimes` design, Risks;
  `macos-arm64-evaluation-assessment-v3.json`).
- The measured Linux helper uses Landlock ABI 6 filesystem rights and both IPC
  scopes (`experiments/microvm/libkrun/src/linux.rs:114-147`). That mediates
  creating and removing a pathname socket, not `connect()` to one, and seccomp
  cannot read a path argument (`evaluate-microvm-runtimes` E3). devcroft closed
  the same gap natively with a mount namespace (`add-mount-isolation` M1).

## Goals / Non-Goals

**Goals:** isolate catalog services in a private companion VM with its own Linux
LTS kernel, memory and writable disk; ensure project-defined service commands
never run in the companion; keep the companion channel inventory closed and
host-initiated; make every server identity other than the workload's
least-privilege client role unreachable through the relay; state the protection
limits honestly; confine each helper and broker before the first guest
instruction or stream, including which Unix sockets it can reach; and account for
both VMs in resource budgets and fleet admission.

**Non-Goals:** launch a companion when no catalog profile is requested; run
`ServiceDecl.command` in the companion; claim that the companion protects data
reachable through the granted client role or keeps service log content
authentic; pre-select `libkrun` before `evaluate-microvm-runtimes` task 4.5;
reclassify the 13.49 GiB preparation builder; claim recovery from host power
loss; upgrade service data implicitly; or require eBPF/Falco or Asterinas.

## Decisions

### C1 — Trusted service catalog and conditional companion VM

In devcroft, `ServiceDecl` (`src/provider/mod.rs:201-220`) carries `command:
String`—a project shell command whose default `working_dir` is the project root.
Running provider-declared service commands in the companion would execute
repository code inside the companion and negate its isolation.

The companion therefore runs **only service profiles from a trusted catalog**
verified against the user-trusted distribution manifest (initially PostgreSQL
and Redis). A profile fixes the service closure, configuration, durability
settings (C8), least-privilege client role and the authentication rules that
keep every other identity off the relay (C4), stop signal, readiness probe, data
format version and default disk size. A project may request a catalog profile by
name through a `devcroft.toml` key whose spelling is decided in task 1.3; an
unknown name fails at layer `config`. A project cannot supply or alter a
profile. Provider services (`ServiceDecl.command`) stay in the workload VM (under
`add-vm-project-runtime`) or are refused by name if requested as catalog
profiles. When a sandbox requests no catalog profile, only the workload VM
starts.

### C2 — Stage 1 substrate: Linux LTS, VMM from 4.5, base chosen by measurement

Both VMs boot the pinned Linux LTS kernel under the VMM selected by
`evaluate-microvm-runtimes` task 4.5. That VMM must carry concurrent connections
on a per-port guest-initiated mapping (workload) and a per-port host-initiated
mapping (companion); one that cannot is a design change, checked in task 1.1.
The companion guest base—NixOS minimal vs. pinned closure + dedicated init (such
as the C `init` measured in `experiments/microvm/guest/init.c` or a static Rust
equivalent)—stays open until both are measured on the same PostgreSQL + Redis
fixture for cold boot, idle memory and disk footprint (task 1.4). Neither base
ships a login daemon, SSH server, keeper or interactive shell. Whichever wins
must carry the filesystem check tool C8 runs.

### C3 — Closed, host-initiated companion channels (`SHUTDOWN` and diagnostics)

The companion's launch inventory contains only host-initiated mappings: one
relay mapping per declared service port, a `SHUTDOWN` control channel and a
diagnostic stream. There are no guest-initiated mappings.

The control channel is held by companion PID 1 (bound before any service starts,
on a vsock port below 1024, with state in `/run/devcroft-init`, `0700`,
`root:root`). It accepts connections only from the host (peer CID 2), so a
service process cannot reach it over local vsock loopback. It accepts **one
opcode, `SHUTDOWN`**, bound to the instance generation, with a fixed frame cap.
The generation is delivered at boot as a non-secret value on the kernel command
line; binding to it keeps a stale controller from an earlier generation from
stopping a new companion. Unknown opcodes, oversized frames, other peers and
generation mismatches close the connection. `BOOTSTRAP_SECRETS` and
`AWAIT_READY` from the first draft are removed: the broker already binds the
companion to a single paired workload, and the broker runs readiness probes
directly.

### C4 — Broker-derived identity; only the client role is reachable

Because the per-instance host broker accepts streams only from the paired
workload VM and connects only to the paired companion VM, the channel itself
establishes instance identity, as `add-vm-project-runtime` R5 does for its own
channels. No per-instance secret is generated or written to images,
`/proc/cmdline`, controller state or logs.

Without a secret, every relayed connection reaches the service from the
companion's relay forwarder (C5), so the server cannot tell a client role from an
administrative one by credential. Giving the workload a client role is therefore
not enough: **every other server identity must be unreachable through the
relay.** Both services' defaults fail open: `initdb` without `--auth` trusts
every role on `127.0.0.1`, and Redis's `default` user has no password and every
command. The profile fixes:

- **PostgreSQL:** administrative roles authenticate only on the companion-local
  Unix socket with `peer`. `pg_hba.conf` admits the client role alone on `host`
  lines (`trust`, since the channel is the identity) and ends with `reject` for
  every other role and database. The client role is `NOSUPERUSER NOCREATEROLE
  NOCREATEDB NOREPLICATION`, with no membership in `pg_execute_server_program`,
  `pg_read_server_files` or `pg_write_server_files`, no untrusted procedural
  languages and no `ALTER SYSTEM`.
- **Redis:** the only ACL user is the restricted client user (`default`,
  redefined), so there is no other identity to authenticate as. It refuses
  `CONFIG`, `MODULE`, `DEBUG`, `ACL`, `SHUTDOWN`, `SAVE`, `BGSAVE` and
  `REPLICAOF`, and the protected-config, debug and module defaults stay disabled.
  Administrative setup comes from the generated configuration and ACL file at
  boot, not from a runtime admin user.

Administrative setup runs from the catalog profile at first boot, over the
companion-local Unix socket, never through the relay.

### C5 — Relay endpoints, per-connection streams and the per-instance broker

Each requested profile declares its TCP port (`5432`, `6379`). The relay has
three parts, and none parses service protocol except the readiness probe (C6):

- **Workload listener.** A relay listener in the workload guest image, started by
  workload init before creation-time setup and again on every boot (including the
  reboot that ends setup, R5), listens on `127.0.0.1:<port>` for each declared
  port and opens a guest-initiated vsock connection on that port's relay mapping.
  It runs inside the workload's trust domain and is not trusted: the broker, not
  the listener, enforces which ports exist. It adds a relay channel to the
  workload's launch inventory, which amends `vm-authority-boundary` ("only its
  declared control, SSH and proxy channels").
- **Broker.** One per instance. For each declared port it accepts connections
  from the workload helper's mapping and opens one connection to the companion
  helper's mapping for the same port, then copies bytes in both directions. There
  is **one host stream per TCP connection**, with no multiplexing, so there is no
  frame format to parse and no stream identifier that could name another
  instance. The broker preserves half-close (`SHUT_WR`), enforces a finite
  concurrent-connection cap and per-direction buffer recorded in the launch plan
  (values chosen and tested at the cap in task 3.2), and closes every stream when
  the workload restarts or reboots. It is launched only with endpoints taken from
  its own instance's controller record.
- **Companion forwarder.** PID 1 starts a relay forwarder under its own
  unprivileged identity, distinct from root and from the service identities. It
  accepts vsock connections only from the host (peer CID 2) on the declared
  ports' relay mappings and connects to `127.0.0.1:<that port>`.

Neither VM has a general network interface (`NIC` or `TSI`). An undeclared port
has no mapping on either side. Concurrent sandboxes use the same guest ports
without host port collisions. An editor's local forward of a guest database port
(`vm-project-runtime`, "SFTP and forwarded service use guest state") reaches the
workload listener and keeps working.

### C6 — Broker readiness probes and error layers

Before workload creation-time setup runs and before interactive or fleet
sessions are admitted, the host broker runs each requested profile's readiness
probe through its declared port. A probe that misses its deadline fails launch
at layer `keeper` (exit 5).

Companion failures keep the five-layer contract (`config | provider | backend |
keeper | ssh`, `src/lifecycle/up.rs:110-118`), mapped as `add-vm-project-runtime`
R6 maps the workload:

| Failure | Layer (exit) |
| --- | --- |
| Unknown or malformed catalog profile request | `config` (2) |
| Catalog entry fails verification; incompatible data format version (C8) | `provider` (3) |
| Companion helper, VMM or host confinement | `backend` (4) |
| Companion PID 1, control or diagnostic channel, companion supervisor, broker, readiness | `keeper` (5), the companion's transport |

### C7 — Graceful stop, profile stop signals and stop classification

PostgreSQL treats `SIGTERM` as Smart Shutdown and waits for open client
connections to close. On `down` or restart, devcroft first stops workload
sessions and **closes every active relay stream at the broker**, then asks the
companion supervisor (C13) to send `SHUTDOWN` to companion PID 1. Once that
request is recorded, the supervisor does not restart the companion. PID 1 stops
each service with its profile's stop
signal (`SIGINT` / Fast Shutdown for PostgreSQL; `SIGTERM` for Redis), waits up
to the profile deadline, runs `sync()`, unmounts the writable disk and
acknowledges completion on the control channel before powering off.

A stop is classified as orderly **only** when that acknowledgement arrives,
correlated with the controller's recorded stop intent and the helper's recorded
identity. The helper's exit status is not evidence: at the libkrun pin a guest
panic also exits 0. Anything else is a forced stop or a crash (C13).

### C8 — Storage durability, size and data format version

**Durability settings are part of the profile**, because the recovery claim
holds only under them: PostgreSQL runs with `fsync=on`, `full_page_writes=on`
and `synchronous_commit=on` by default; Redis runs with `appendonly yes` and
`appendfsync always`, the configuration 3.7.1 measured. The client role cannot
change Redis's (C4 refuses `CONFIG`). `synchronous_commit` is a PostgreSQL
setting a client may lower for its own session; commits made that way are
outside the claim.

`companion-rw.img` honours guest flushes with full flush semantics (libkrun's
`SyncMode::Full` in the measured helper, or the equivalent in the selected VMM).
For helper termination, the scope of the claim, data the helper has already
written to the host file survives the process either way; full flushing matters
for host crashes, which are not claimed. It is kept because the measured recovery
used it, and its throughput cost for PostgreSQL commits is recorded in task 1.4.

`companion-rw.img` has a finite size: the profile's default or the user's
setting, never unbounded. A full disk is a companion service failure that
`status` reports. PostgreSQL stops when it cannot write WAL, and that is never
silent.

If the companion did not stop in order (C7), status records a forced stop or a
crash, and the next start, automatic (C13) or not, runs a pre-mount filesystem
check and service recovery before the relay reopens. A check that needs manual
repair stops there and is reported rather than retried. The claim covers helper termination only
(`evaluate-microvm-runtimes` 3.7.1); none is made for host power loss (3.7.2).

`companion-rw.img` records the profile and data format version that wrote it
(such as the PostgreSQL major version). If an updated catalog entry selects an
incompatible version, `up` refuses at layer `provider`, names both versions and
leaves the data untouched. devcroft performs no upgrade; the user can dump the
data through the relay with the service's own client before replacing it.

### C9 — Published protection claim and limits

Documentation, `status` and `docs/threat-model.md` state both what the companion
protects and what it does not:

- **Protected from the paired workload:** the companion's kernel, memory,
  service binaries, server configuration, administrative identities, raw disk
  blocks, and clean service startup/shutdown lifecycle (workload OOM or root
  kills do not kill the companion).
- **Not protected from the paired workload:** data the workload's granted client
  role is allowed to read, modify or drop through the service protocol, including
  objects it creates (functions, triggers) that persist across a workload
  rebuild; exfiltration through service responses over allowed workload channels;
  and the authenticity of service log lines (only structured events emitted by
  companion PID 1 have integrity against the workload, C11).

### C10 — Per-role pre-vCPU host confinement and Unix-socket reach

Every VMM helper (workload, companion, preparation), every per-instance broker
and every companion supervisor applies host confinement in-process after opening
its descriptors and
before the VMM is built and the first guest instruction runs, or before its
first stream. Confinement governs which Unix sockets a role can **reach**, not
only which it can create or remove. Each role gets its own socket directory, each
`sun_path` within the 103-byte limit:

| Directory | Bound by | Reachable by |
| --- | --- | --- |
| `w/` | workload helper (control, SSH, proxy) | workload helper, controller |
| `wr/` | broker (one listener per declared port) | workload helper (connect only) |
| `cr/` | companion helper (relay mapping per port) | broker (connect only) |
| `cc/` | companion helper (control, diagnostics) | companion supervisor only |
| `cs/` | companion supervisor (request socket, restart records, diagnostic buffer) | controller only |
| `meta/` | controller records | controller only |

- **macOS:** a deny-default Seatbelt profile (`sandbox_init`) that also limits
  outbound Unix-socket connections to the role's paths above.
- **Linux:** Landlock ABI ≥ 6 plus a `seccompiler` kill-on-miss syscall
  allowlist on every VMM thread (`evaluate-microvm-runtimes` E3), **plus an
  unprivileged user + mount namespace** in which only the role's own directories,
  artifacts and private disk are visible. Neither Landlock ABI 6 nor seccomp
  mediates `connect()` to a pathname socket, and all helpers run under one host
  UID. This is the `add-mount-isolation` technique. Task 4.2 measures it with
  `/dev/kvm`. If it cannot carry the helper, separate host UIDs per role (which
  need privileged setup) are the fallback, and until one is measured the Linux
  row fails closed rather than claiming socket separation.

A helper inherits its parent's confinement and can only narrow it, so the
companion supervisor's confinement covers the companion helper's grants plus
launching it, and nothing on the workload side.

`nono` may be used only where it reproduces this exact profile. Production
adoption waits for the owner's `author_claimed_security_boundary` decision under
`evaluate-microvm-runtimes` task 4.5.

### C11 — Host-direct companion diagnostics and optional eBPF

Companion PID 1 writes its own structured lifecycle events and service output to
the host-initiated diagnostic stream, which the host stores in a size-bounded
buffer without routing through the workload VM. The companion supervisor (C13)
holds the only diagnostic connection, opened before any service starts on every
boot, and parses only the stream's framing. The CLI acting as controller exits
after `up`, so it cannot be the reader. PID 1 accepts the connection only from
the host and refuses a second one while it is open, and a supervisor that cannot
open it fails launch at layer `keeper`.

Service output is labelled distinctly from PID 1 events. The workload can make a
service write output through the protocol, so **PID 1's own events have reserved
capacity**: only service output is dropped under pressure, and drop counts are
themselves PID 1 events. PID 1 never blocks on the stream, so `SHUTDOWN`
completes under a flood. Optional in-guest eBPF/Falco monitoring is non-gating,
outside the security claim, and requires a separate diagnostic Linux kernel
profile (`CONFIG_BPF_SYSCALL`, `CONFIG_TRACEPOINTS`, `CONFIG_FTRACE`,
`CONFIG_DEBUG_INFO_BTF`).

### C12 — Dual-VM resource budgets and fleet admission

Launch plans, `status` and fleet admission account for both helpers, both
guests, the broker, the companion supervisor and `companion-rw.img` (`2N`
microVMs for `N` service-backed
sandboxes), reporting workload and companion separately and together. The
13.49 GiB preparation builder artifact is not retroactively reclassified
(`macos-arm64-evaluation-assessment-v3.json`, which retains v2's statement).

Service data grows with use, so the 8.00 GiB per-environment ceiling is checked
against the companion's **fixed footprint**: its image plus an initialized, empty
data directory. The data itself is bounded by the disk size (C8), which `status`
shows. The measured single environment is 6.59 GiB, leaving 1.41 GiB. If the
fixed footprint does not fit, the owner decides whether to revise the ceiling
for service-backed sandboxes before adoption.

### C13 — Automatic companion restart and the companion supervisor

The owner decided (2026-11-13) that a crashed companion restarts automatically.
Something on the host must notice the crash after `up` has exited, and the CLI
acting as controller does not stay running (`add-vm-project-runtime` R6). So `up`
starts a **companion supervisor** per instance, with the instance's lifetime, as
R6 gives the host proxy; it is not a global daemon. It:

- launches each companion helper incarnation;
- holds the companion's control and diagnostic connections (C3, C11);
- carries out the controller's stop request (C7) through a request socket only
  the controller can reach (C10);
- writes restart records host-side, so neither guest can alter them.

It never carries relay streams. Merging it with the broker was rejected: the
broker handles workload-controlled bytes, and its confinement must stay narrower
than what relaunching a helper needs (C10).

**Detection.** A helper exit with any status and no acknowledged `SHUTDOWN`, or
PID 1 not answering on the control channel, is a crash (C7).

**Restart sequence.**

1. The broker refuses new relay streams and closes open ones, and `status` shows
   the companion as restarting.
2. The supervisor relaunches the helper and records a new companion boot
   identity, which is what C7 correlates against.
3. Companion init runs C8's pre-mount check and service recovery.
4. The supervisor reopens the diagnostic connection.
5. The broker accepts streams again only after its readiness probe passes (C6).

The workload is not restarted; its clients see refused connections until then.

**Bounds.** Restarts back off. After a bounded number of consecutive failed
restarts within a window, the supervisor stops trying and `status` reports the
companion as crashed with the last reason; the next `up` or an explicit restart
tries again. The backoff, count and window are recorded in the launch plan.

**No restart** after a recorded stop request, or for failures a restart cannot
fix: an incompatible data format version, a filesystem check that needs manual
repair, a catalog verification failure or a host confinement failure. These leave
the companion stopped, with the reason in `status`.

## Risks / Trade-offs

- **2N helpers and tighter per-environment storage** → launch the companion only
  when catalog profiles are requested (C1), measure the companion base footprint
  (task 1.4), and surface the combined budget without reclassifying retained
  artifacts (C12).
- **Least-privilege client roles cannot run superuser migrations** → catalog
  profiles pre-enable declared extensions during PID 1 first-boot administrative
  setup (C4).
- **`appendfsync always` and full flushing cost throughput** → kept because the
  recovery claim depends on them; the cost is measured (task 1.4) and published,
  not traded silently for speed (C8).
- **The Linux mount namespace is unmeasured for VMM helpers** → measured in task
  4.2 with a UID fallback; the Linux row fails closed until one works (C10).
- **Automatic restart adds a long-lived host process and can loop** → one
  supervisor per instance with the instance's lifetime, confined to the companion
  side; bounded backoff; no restart for failures a restart cannot fix (C13).
- **Provider-declared services do not get companion isolation** → stated clearly
  in CLI output and docs; arbitrary project commands belong in the workload VM
  (C1, C9).

## Migration Plan

Additive to the VM chain. Native sandboxes (and `add-macos-service-vm`) are
unchanged. Before enabling by default for service-backed VM sandboxes, complete
`evaluate-microvm-runtimes` task 4.5, record the companion base choice from task
1.4, and reconcile the amended requirements in `add-vm-project-runtime`,
`add-private-local-environments` and `gate-vm-fleet-on-verified-environments`
(task 7.1).

## Open Questions

- Which companion guest base (NixOS minimal vs. pinned closure + dedicated init)
  wins the task 1.4 measurement.
- The `devcroft.toml` spelling for requesting catalog profiles (task 1.3).
- Whether the owner raises the 8.00 GiB per-environment storage ceiling for
  dual-VM sandboxes once the companion's fixed footprint is measured.

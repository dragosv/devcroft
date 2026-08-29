# Design — Linux Agent Fleet

## Context

The sandbox crate covers the intra-agent layer. This change builds the
inter-agent layer. The recurring temptation is to reach for `bubblewrap` and
compose flags; the decisions below explain why that path is rejected and what
replaces it.

Terminology note: **netns** = network namespace. It is stack isolation, not a
filesystem. Each netns has its own port table, which is the mechanism that makes
N agents each bind `:5432` without collision.

---

## D1 — The supervisor owns the process; the sandbox crate is a library

**Decision.** devcroft performs the `clone`/`fork` and `exec` itself and applies
the sandbox ruleset to the child. It does not delegate process spawning.

**Rationale.** Fleet requires work *between* forking and applying restrictions:
entering namespaces, performing mounts, joining a cgroup. A spawn-and-forget API
gives no seam for any of that, and no handle for lifecycle, metrics or
snapshot coordination.

**Confirmed against the library at the pinned version.** `PreparedLandlockSandbox`
is described as a ruleset whose allocation and path opening happen in the parent,
before a raw-cloned child exists — exactly this split. `prepare_seccomp_with_abi`
prepares the policy for allocation-free application on the child side, and
`RawSandboxError` / `RawSandboxStage` give fixed-size, child-safe error reporting
from operations that run after clone. The library is designed for this pattern;
no upstream work is required.

## D2 — The re-executed init helper is PID 1 of the agent's namespace

**Decision.** The supervisor builds the Landlock ruleset in the parent, then
`clone`s the child into new **user, mount, PID, network, IPC and UTS**
namespaces and immediately re-execs its own binary as a hidden `devcroft-init`
subcommand. That helper is single-threaded, receives configuration over a pipe
and inherited file descriptors, and performs: identity mapping handshake →
mounts → add namespace-local rules → restrict_self → seccomp → start the
keeper.

`devcroft-init` **is PID 1** in the new PID namespace: it reaps descendants and
starts the keeper with its sockets inherited, rather than `exec`ing itself away.
That is forced rather than chosen — if PID 1 exits, the kernel `SIGKILL`s the
entire PID namespace, so the helper cannot hand off by replacing itself, and
something must reap orphans or the namespace accumulates zombies.

**Identity handshake.** The parent writes single-entry UID and GID maps
(`0 -> ` the host's real UID/GID), with `setgroups=deny` written *before*
`gid_map` — mandatory for an unprivileged writer since kernel 3.19. The child
blocks until this succeeds, then continues. A single-entry identity map needs no
`/etc/subuid` range, no `newuidmap` helper, and keeps files the agent creates in
its workspace owned by the real user on the host, which is what makes the
resulting clone reviewable and committable without a chown dance.

**Confirmed by the netns spike, and partly implemented.** The `net` half of this
is built (`src/fleet/netns.rs`): unprivileged `CLONE_NEWUSER | CLONE_NEWNET`
works, and the child holds `CAP_NET_ADMIN` inside the new user namespace
immediately — no uid-map write is needed before configuring the interface. The
spike also found that a fresh netns's loopback is `DOWN`, which the helper must
fix before any service binds; see D5.

**Rationale.** Landlock rules bind to inodes, not path strings, so most of the
ruleset can be built in the parent where allocation is safe, then carried across
the fork as a file descriptor (not `CLOEXEC`). But `fork()` in a multi-threaded
process followed by allocating code is unsafe — the allocator may hold locks
owned by threads that do not exist in the child. Re-exec is the standard
mitigation; it is why runc has an init helper.

Paths that exist only inside the new mount namespace (the bind target, tmpfs,
remounted `/proc`) cannot be opened from the parent. Those few rules are added
post-fork, which is a bare `open()` plus `landlock_add_rule` on the existing
ruleset — no allocation, before `restrict_self`.

**Ordering constraint.** Mounts must happen before `restrict_self`. Not because
paths "disappear" — inode-bound rules survive bind mounts — but because a
restricted process can no longer read and mount what it needs to build the
world.

**Library support, and why re-exec is now settled rather than open.**
`PreparedLandlockSandbox` and `PreparedSeccompNotifyFilter` are both built in
the parent and carried across the clone; the child-side apply is allocation-free
and reports through `RawSandboxError`. The library genuinely does support raw
clone directly, and an earlier version of this decision therefore left re-exec
as "measure before committing to it".

That is now decided: **re-exec is required**, and not because of the library.
The child must do namespace entry, the identity handshake, a mount plan, become
PID 1 and reap, and receive an inherited listener — none of which the library's
child-safe operations cover, and all of which need ordinary allocating code in a
process that must not have inherited a multi-threaded allocator's locks. The
library removing one reason for re-exec does not remove the other five.

## D2a — Mount view: two strategies, one of them explicitly selected

**Decision.** Both strategies preserve the same three contracts — the agent's
clone is read-write at the fixed path `/workspace`, provider runtime paths are
read-only, and workspace files are owned by the real host user. They differ in
what is *visible*:

- **Host-root plus Landlock (MVP default).** The host's root stays visible in
  the agent's private mount namespace, with private `/workspace`, `/tmp`,
  `/proc` and `/dev`. Landlock refuses every path not explicitly granted:
  denied paths remain *visible* but fail with `EACCES`.
- **Explicit minimal root (post-spike).** A fresh private root containing only
  `/workspace`, the resolved runtime paths, and the minimum read-only system
  paths the provider needs. The agent cannot enumerate what was omitted —
  closer to what bubblewrap gives.

The selected strategy is **recorded in fleet state and reported by `status`**,
and fleet never silently downgrades from minimal-root to host-root.

**Rationale.** Minimal-root is the better end state and the more expensive one:
it needs a compatibility inventory per distribution × provider — dynamic loader,
merged `/usr` symlinks, NSS, DNS, CA certificates, `/dev` nodes, and real
toolchains — and getting it wrong produces failures that look like the project's
rather than devcroft's. Host-root-plus-Landlock reaches a working agent sooner
with the same *access* boundary and a weaker *visibility* one.

Recording which one is in force matters because the difference is invisible from
inside a correctly-configured agent: both deny the same reads. A silent
downgrade would therefore be undetectable by the person relying on it, which is
the same reason `policy::degraded` names an unenforceable aspect rather than
quietly applying the fallback.

## D3 — No bubblewrap

**Decision.** Namespaces are created via `clone`/`unshare` directly.

**Rationale.** bwrap is a binary that calls the same syscalls. Once a supervisor
exists for cgroups and lifecycle, wrapping it adds a layer that removes control:
flags instead of calls, errors reduced to exit codes, and no seam to interleave
anything between unshare, mount and `restrict_self`. It is a good fit for
single-shot invocation and a net loss for a supervisor.

Secondary: bwrap requires unprivileged user namespaces, which Ubuntu 23.10+
restricts via `apparmor_restrict_unprivileged_userns` and permits only through
the distribution's shipped AppArmor profile. Owning the code makes that a
detection-and-diagnostics problem rather than a dependency problem.

## D4 — The egress proxy runs outside the agent's namespaces

**Decision.** One proxy instance per agent, hosted as a task in the supervisor on
the host, reached from inside the netns via a forwarded port.

**Rationale.** Two properties fall out for free:

1. **Attribution.** The supervisor knows which agent called, from which listener
   the connection arrived on. No in-band identification protocol.
2. **Credential safety.** A proxy running inside the sandbox holds real
   credentials in the same Landlock domain as the agent. Landlock's ptrace
   restriction only blocks tracing *less*-restricted processes; within one
   domain it is permitted. The agent could then read `/proc/<pid>/mem` and
   recover exactly the tokens the phantom-token design exists to hide. Hosting
   the proxy outside the domain and outside the pid namespace removes the attack
   entirely rather than mitigating it.

**Depends on `add-egress-proxy`.** The domain-filtering proxy this decision
places outside the sandbox does not exist yet — filtering is declared in the
manifest and compiles to a blanket network block. This decision is about
*placement*; the component itself is that change's subject. Fleet's
`agent-networking` requirements assume it and do not restate it.

## D5 — Rootless connectivity: slirp4netns baseline, proxy-only by policy

**Decision.** `slirp4netns` is the MVP baseline, **conditional on a live probe
of the exact flags fleet needs** (`--disable-host-loopback`, explicit inbound
forwarding, no automatic port forwarding). `pasta` remains a future option after
equivalent validation across distributions and on teardown.

**The network helper is not an egress firewall.** It provides a stack; it does
not decide what may be reached. That role belongs to the seccomp proxy-only
policy (D9), which permits the agent's local proxy endpoint and its declared
listener ports and nothing else. Conflating the two would be the classic
mistake: a userspace network helper looks like a boundary and is not one, since
the workload can open arbitrary sockets through it unless separately prevented.

**Preflight probes behaviour, not presence.** Checking that the binary exists
proves nothing about the flags this version accepts. The preflight runs a real
probe in a disposable namespace and verifies: the required flags are accepted,
the proxy endpoint is reachable, host loopback is refused, declared inbound
forwarding works, and teardown leaves nothing behind. A host that fails the
probe refuses fleet rather than running with a weaker network model.

**Status: baseline selected, comparison not yet runnable here.**

An empty netns has no route out. Something must provide connectivity from user
space, and both candidates also perform selective port forwarding in each
direction, which serves the proxy reach-back (D4) and any host-side service
mapping (D8).

The selection criteria remain: forwarding semantics and flag stability,
throughput on loopback-heavy workloads, packaging across target distributions,
and teardown behaviour. slirp4netns is the *baseline* on packaging and prior art
in adjacent rootless-container tooling; it is not yet a *measured* winner here,
because neither candidate can run in this devcontainer at all (below). The
preflight above is what makes the baseline safe to name in advance: a host where
the chosen helper does not behave as required refuses fleet rather than
silently degrading.

### Spike results (measured, this devcontainer, kernel 7.0.14)

**1. Unprivileged user+net namespaces work.** `unshare --user --net
--map-root-user` succeeds; `/proc/sys/user/max_user_namespaces` is 48184 and
there is no `unprivileged_userns_clone` restriction here. The gate for
everything below is open.

**2. This decision's own premise is misleading, and the correction matters.**
"An empty netns has loopback" is true only in the sense that a `lo` *device*
exists — it is `DOWN` with no address. Measured consequence:

```
fresh netns, lo untouched:  bind(127.0.0.1:5432) OK, connect() → ENETUNREACH
after `ip link set lo up`:  bind OK, connect OK
```

A service would therefore **start, report itself healthy, and be silently
unreachable** — the precise failure shape `add-flox-services` exists to
prevent. Bringing `lo` up requires no external tool, no TUN device, and no
elevated privilege beyond the user namespace the agent already has. It belongs
to namespace construction (task group 3), not connectivity (group 4), and was
not previously anyone's task. Added as 3.x.

**3. Both candidates are unavailable here, for the same reason.** `pasta` and
`slirp4netns` both fail with `open("/dev/net/tun"): No such file or
directory`; this devcontainer has no `/dev/net` at all. The throughput and
forwarding-semantics comparison this decision calls for **cannot be run in
this environment** — developing fleet's egress path needs the devcontainer to
pass the device through. That is a prerequisite to finishing D5, not a result
of it.

**4. Service ports do not depend on any of that.** Demonstrated directly:
two concurrent netns, each running `ip link set lo up` and binding
`127.0.0.1:5432`, each reaching its own listener, with the host's own 5432
left free.

```
agent a17: bound 5432, reached postgres-of-a17
agent a18: bound 5432, reached postgres-of-a18
host 5432 still free — agents did not take it
```

No pasta, no slirp4netns, no TUN device. This is the whole of what
`specs/service-ports/spec.md` promises for the in-namespace case, and it is
already achievable. **D5 gates egress — reaching the proxy, reaching a
registry, and the host-side mapping — not port isolation.** An agent with
loopback up and no forwarding is fully network-isolated except its own
loopback, which is a safe default rather than a broken state.

The practical consequence for sequencing: `service-ports` and the in-namespace
half of fleet can be built and tested before D5 resolves. Only the optional
host mapping waits.

## D6 — One delegated cgroup v2 subtree, one leaf per agent

**Decision.** Fleet runs as a **systemd user service with `Delegate=yes`**. The
supervisor creates an empty internal `fleet` cgroup inside the delegated
subtree, then one **domain leaf cgroup per agent**.

Each agent's leaf contains that agent's init helper, keeper, sessions, services
and local forwarder. It deliberately does **not** contain the host-side egress
proxy or the supervisor itself — the proxy sits outside the agent's policy
domain by D4, and putting it inside the agent's cgroup would let the agent's
resource pressure (or `cgroup.kill`) take down the component that is supposed to
be filtering it.

**Mechanics that are easy to get wrong, so they are written down:**

- Discover the cgroup via `/proc/self/cgroup`; do not assume a fixed systemd
  path, which varies by distribution and by whether a user manager is present.
- Enable `cpu`, `memory` and `pids` controllers top-down through
  `cgroup.subtree_control`.
- Per leaf: `memory.max`, `memory.swap.max=0`, `memory.oom.group=1`,
  `cpu.weight`, `pids.max`. `io.weight` only where the `io` controller is
  available — its absence is a **named degraded capability**, not a reason to
  abandon the other limits.
- The `fleet` node stays process-free: cgroup v2's no-internal-process rule
  means a populated internal cgroup cannot distribute domain controllers to its
  children.
- Leaves are never `threaded`: `cgroup.kill` must terminate every descendant,
  and the supervisor waits for `cgroup.events: populated 0` before removing the
  leaf.

**A working systemd user manager and a delegated unified cgroup v2 subtree are
hard MVP requirements. There is no manual-delegation fallback.**

**Rationale.** An earlier version preferred `systemd-run --user --scope` with
"manual cgroup2 delegation as the fallback". Dropping that fallback is a
deliberate narrowing: reimplementing controller enablement, delegation
ownership and `cgroup.events` semantics by hand reproduces exactly the logic
systemd already owns, in a place where getting it subtly wrong yields limits
that appear configured and do not hold. A hard requirement with a clear
preflight failure is more honest than a fallback that silently under-enforces.

The three things this buys remain the reason it is the foundation:

- **Limits.** Without them, one runaway build blocks the fleet. This is the
  single most likely failure mode at N ≥ 3.
- **Teardown.** `cgroup.kill` terminates the whole subtree atomically. No orphan
  hunting, no reparenting races.
- **Observability.** `memory.current`, `cpu.stat` and `cgroup.events` provide
  `devcroft ps` and exit detection from the same mechanism.

## D7 — Git: shared bare mirror, per-agent clone with `--reference`

**Decision.** A single bare mirror per upstream repository; each agent gets its
own clone referencing it. Automatic GC is disabled on the mirror.

**Rationale.** Worktrees share the object store and refs. `index.lock` is
per-worktree but `packed-refs` is not, and git takes locks without retrying — an
agent receives a spurious failure and reacts to it. `--reference` keeps disk
usage close to worktrees while giving each agent an independent ref namespace.

Agents do not push directly to the upstream remote. Without an integration step,
snapshot/rollback is meaningless once an agent has published.

## D8 — Port model: declared in-namespace port plus optional host mapping

**Decision.** A service declares the port it binds inside its namespace. A
separate optional field maps it to a host port for developer access.

**Rationale.** On Linux the in-namespace port is authoritative and identical for
every agent; the mapping is allocated per agent. On macOS there is no netns, so
the mapping is the only mechanism and the in-namespace port is advisory. One
config schema serves both, and the macOS path needs it regardless of fleet: two
projects open simultaneously collide at N=2, which is precisely what people do
with devcontainers.

## D9 — The proxy-only seccomp filter is mandatory; general syscall hardening is not

**Decision.** Where runtime egress is requested, fleet installs the narrow
**proxy-only seccomp-notify filter** `add-egress-proxy` provides. General
syscall-surface hardening remains deferred to `add-syscall-filtering`.

**This reverses the previous decision**, which deferred seccomp from the fleet
MVP entirely on the reasoning that "the boundary is Landlock plus the network
policy". That reasoning does not survive D5: once a userspace network helper is
providing a general stack inside the agent's namespace, proxy environment
variables are *cooperative*. A workload that simply ignores `HTTPS_PROXY` and
opens its own socket reaches whatever the helper can route to. Landlock's
network rules are port-based and cannot express "only this endpoint"; the
filter can, and nothing else in the design can.

So the filter is not extra hardening layered on a working boundary — it **is**
the boundary for runtime egress. Direct sockets fail closed; only the agent's
local proxy endpoint and its declared listener ports are permitted.

What stays deferred is unchanged and still correct: broad syscall reduction
targets kernel attack surface, which matters against an exploit rather than
against an agent behaving badly, and shipping it as an unimplemented optional
requirement would misrepresent the fleet story.

**Phase-0 gate — the notification listener handoff.** The installed filter traps
`sendmsg`, so the helper cannot assume it can pass the newly created listener FD
over an ordinary control socket *after* installation. Fleet starts no workload
until a validated bootstrap/handoff mechanism transfers that FD to the host's
proxy loop. `nono` documents two candidate paths (a short `CLONE_FILES`
bootstrap, or the pidfd-based route); neither is validated here yet, and this is
the single blocking item before any proxy work begins.

**One ordering hook is still required:** the init helper's step sequence must
leave a seam between applying the sandbox ruleset and starting the workload, so
`add-syscall-filtering` has somewhere to insert later. That ordering is
specified in `sandbox-runtime`.

## D10 — Pin the sandbox crate exactly

**Decision.** Pin to an exact version; treat upgrades as scheduled work with an
integration test that exercises behaviour, not just types.

**Rationale.** Pre-1.0 crates get breaking changes on the minor version. Given
the release cadence, this recurs.

---

## Rejected Alternatives

**Bubblewrap as the sandbox executor.** See D3.

**Proxy inside the netns.** Simpler to build; loses attribution and exposes
credentials to `ptrace` from within the same Landlock domain (D4).

**`--unshare-net` with no user-space networking.** Produces a dead network, not
a filtered one. This is a real trap: the flag looks like hardening and is a
functional break.

**Relying on egress filtering as an exfiltration guarantee.** A domain allowlist
constrains destinations; it does not prevent exfiltration. Any allowlisted
endpoint that accepts a POST is a channel — an allowed `github.com` means
gists, branches and issue bodies. State the property as "egress is constrained
to allowlisted destinations" and size the allowlist accordingly.

---

## Open Questions

1. **Seccomp notification listener handoff — blocking.** The proxy-only filter
   traps `sendmsg`, so the listener FD it returns cannot be passed over an
   ordinary control socket after installation. A validated bootstrap must exist
   before any proxy work starts (D9). This is the one item that blocks rather
   than merely informs.

2. **Snapshot cost and backend — deferred out of the fleet MVP.** If overlayfs,
   unprivileged use needs kernel 5.11+; if copy-based, it becomes the dominant
   startup cost at N agents on a large worktree. Deferred rather than answered:
   per-agent clones (D7) give isolation without it, and snapshotting is an
   optimisation on top.

3. **Kernel floor.** Decide the minimum Landlock and seccomp-notify ABI, and
   what happens below it. **Fleet does not silently fall back** to host
   networking, shared PIDs, or absent resource control — a host below the floor
   refuses fleet, consistent with D5's and D6's preflights.

4. **Shared cache policy.** A shared writable package cache is a channel between
   agents that are otherwise isolated. Decide whether read-only sharing plus
   per-agent overlays is needed, or whether per-agent caches are affordable.
   This interacts with `sandbox-provisioning`'s identical open question and
   should be settled once, in whichever lands first.

**Resolved since the previous version of this list.** The D1/D4 API shape is
confirmed against the pinned library; pasta-vs-slirp4netns has a named baseline
with a behavioural preflight (D5); the Nix-daemon-under-concurrency question is
answered by `sandbox-provisioning`'s P2a/P2b — agents get read-only resolved
runtime paths and no daemon socket, so there is no concurrent-GC hazard to
manage here; and the two-tier model collapsed on its own when
`remove-gvisor-backend` removed the second tier.

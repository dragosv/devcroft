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

## D2 — Restrictions are applied by a re-executed single-threaded init helper

**Decision.** The supervisor builds the Landlock ruleset in the parent, then
`clone`s and immediately re-execs its own binary as a hidden `devcroft-init`
subcommand. That helper is single-threaded, receives configuration over a pipe
and inherited file descriptors, and performs: unshare → mounts → add
namespace-local rules → restrict_self → optional seccomp → exec the agent.

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

**Library support.** `PreparedLandlockSandbox` and `PreparedSeccompNotifyFilter`
are both built in the parent and carried across the clone; the child-side apply
is allocation-free and reports through `RawSandboxError`. The re-exec decision
below is therefore about *devcroft's* multi-threaded runtime, not about the
library — which already supports raw clone directly. If the supervisor's own
setup work after clone stays within what the library's child-safe operations
allow, re-exec may be avoidable. Measure before committing to it.

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

## D5 — Unprivileged connectivity: pasta (passt) or slirp4netns

**Status: OPEN — prototype before finalising the spec.**

An empty netns has loopback and no route out. Something must provide
connectivity from user space. Both candidates also perform selective port
forwarding in each direction, which serves both the proxy reach-back (D4) and
service exposure (D8).

Selection criteria: forwarding semantics and flag stability, throughput on
loopback-heavy workloads, packaging across target distributions, and behaviour
on process teardown. This is the only piece of the architecture with no prior
art inside the project — build a spike first.

## D6 — One delegated cgroup v2 scope per agent

**Decision.** Each agent gets a delegated scope with `MemoryMax`, `CPUWeight`,
`IOWeight` and `pids.max`.

**Rationale.** This is the foundation, and it yields three things at once, not
just limits:

- **Limits.** Without them, one runaway build blocks the fleet. This is the
  single most likely failure mode at N ≥ 3.
- **Teardown.** `cgroup.kill` terminates the whole subtree atomically. No orphan
  hunting, no reparenting races.
- **Observability.** `memory.current`, `cpu.stat` and `cgroup.events` provide
  `devcroft ps` and exit detection from the same mechanism.

`systemd-run --user --scope --slice=` is the preferred path where a user manager
is present; manual cgroup2 delegation is the fallback.

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

## D9 — Syscall filtering is out of scope for this change

**Decision.** No seccomp in the fleet MVP. Deferred to `add-syscall-filtering`.

**Rationale.** The boundary is Landlock plus the network policy. seccomp reduces
kernel attack surface, which matters against an *exploit* — third-party code
from `npm install` or a build script — not against an agent behaving badly. With
the proxy outside the sandbox (D4), `ptrace` and `process_vm_readv` stopped being
urgent, and those were the one genuinely load-bearing entry on the list.

Shipping it as an unimplemented optional requirement would be worse than
omitting it: it reads as a gap in the fleet story when it is not one. The
evaluation and the implementation constraints live in the separate change, which
depends on this one.

**One hook is required here:** the init helper's step sequence must leave a seam
between applying the sandbox ruleset and exec'ing the agent command, so the
later change has somewhere to insert. That ordering is already specified in
`sandbox-runtime`.

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

1. **D1/D4 API shape** at the pinned crate version — blocking.
2. **pasta vs slirp4netns** (D5) — needs a spike.
3. **Snapshot backend.** If overlayfs, unprivileged use requires kernel 5.11+.
   If copy-based, it becomes the dominant startup cost at N agents on large
   worktrees. Determines whether agent startup is interactive-fast.
4. **Nix daemon under concurrency.** `/nix/store` read-only is fine, but the
   daemon socket must be bind-mounted into each namespace and GC roots must be
   per-agent, or a GC triggered by one agent collects another's live paths.
   Needs an explicit concurrency test, not an N=1 test.
5. **Does the two-tier model survive?** Two axes now exist (platform,
   single/fleet). The tiers may be better expressed as the second axis.
6. **Kernel floor.** Landlock network rules require 6.7+; `IOCTL_DEV` is ABI v5
   (6.10); signal and abstract-socket scoping is ABI v6 (6.12). Decide the
   supported floor and the degradation behaviour below it.

# Prior art

Projects devcroft has read and taken something from — mechanisms,
sequences, or the shape of a problem — without depending on them.

The pattern is deliberate and predates this file. devcroft takes
*techniques* rather than tools: `fleet::netns` reimplements what a helper
binary would do in ~50 lines of `unshare` and an ioctl, and
`add-mount-isolation` M2 refuses bubblewrap for the same reason. Two
standing requirements make that a rule rather than a preference — the
keeper "SHALL NOT be executed as a child of a separate sandboxing binary",
and "the process tier requires no external backend binary". So a project
here is a source of ideas, not a candidate dependency, unless the entry
says otherwise.

Recording what was taken, and from where, is the point: an idea whose
origin is lost gets re-litigated.

---

## sandlock — <https://github.com/multikernel/sandlock>

Apache-2.0, Rust. A process-based Linux sandbox targeting, in its own
words, *"strict confinement without image builds or root privileges"* —
the same gap devcroft's own README describes between containers and
nothing. It combines Landlock, seccomp-BPF and seccomp user notification
behind a supervisor process.

**Closest in scope to `nono-cli`, not to devcroft.** It confines a
command; it does not provision a reproducible environment, supervise
services, or expose SSH. Same relationship, same conclusion:
`docs/comparison.md`'s reasoning about nono-cli applies here unchanged.
Not a backend candidate.

### Taken: the seccomp notification FD handoff

`add-linux-agent-fleet`'s D9 declared a hard gate — no proxy work until a
listener-FD handoff is validated — because "the proxy-only filter traps
`sendmsg`, so the listener FD cannot be passed over an ordinary control
socket after installation".

sandlock resolves this by **ordering**, not by a bootstrap trick: fork,
the child installs the filter and receives the listener FD, the child
transmits that FD to the parent *as part of installation* — before
anything is trapped — then blocks on a "ready" signal until the parent's
supervisor is live on it, and only then execs.

The chicken-and-egg exists only if the FD is passed *after* the filter is
enforcing. That suggests devcroft's blocker is specific to nono's choice
to trap `sendmsg`, not inherent to seccomp-notify. Recorded against the
spike task so it is read before that work starts, and it compounds with a
separate finding that the filter may not be needed at all.

### Taken: Landlock IPC scoping, which devcroft already had access to

sandlock uses `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET` (ABI v6, kernel 6.12).
**nono already exposes it** — `IpcMode::SharedMemoryOnly` is what makes it
request `Scope::AbstractUnixSocket` — and devcroft never sets it.

That is half of a gap devcroft had already documented from the other
direction. `docs/known-gaps.md` recorded that Landlock does not mediate
unix sockets, framed entirely around *pathname* sockets and the nix daemon.
Abstract sockets are the other kind, they *are* expressible, and the
capability was sitting unused in a dependency. `add-backend-capabilities`
had even flagged `IpcMode` as `not-adopted` — the matrix worked, and
nobody joined it to the finding.

Unmeasurable on this devcontainer, which has no abstract sockets at all
(`ss -xl` shows zero); real on a desktop, where dbus, X11, PipeWire and
systemd-journald all use them.

### The capability is wanted and already planned; the mechanism is not

sandlock stages writes to a working directory in an upper layer via
seccomp interception of filesystem syscalls, committing on clean exit —
*"no mount namespace, no user namespace, no root"*.

First, a confusion worth heading off: this is **not** an alternative to
`add-mount-isolation`. That change hides paths so they cannot be named;
this protects a directory from modification. Different goals, and taking
one says nothing about the other.

**Rollback itself is already on the roadmap**, via a different mechanism:
`add-linux-agent-fleet` task 34 is "confirm the snapshot layer is the
content-addressable `undo` module rather than an overlay", and that
change's open question 2 defers snapshotting on the reasoning that
per-agent clones (D7) give isolation without it and snapshotting is an
optimisation on top. So the question is not whether devcroft wants an
agent's mess to be undoable — it does — but which mechanism buys it.

**Why not this one, specifically.** Three costs, and the first is the one
that decides it:

- **It needs a supervisor outside the sandbox, live for every syscall, for
  the sandbox's whole life.** devcroft's keeper is *inside* the boundary
  by design — it self-restricts. A seccomp-notify supervisor cannot be.
  That introduces a component whose death breaks the sandbox's ability to
  *compute*, which devcroft currently has none of: if the egress proxy
  dies today, egress fails closed and everything else keeps working.
  Trading that property for rollback is a bad trade when cheaper rollback
  exists.
- **Per-syscall cost, on a workload of compilers.** sandlock benchmarks
  Redis; devcroft's characteristic workload is a build touching hundreds
  of thousands of files, where a userspace round-trip per `open` is a
  different order of cost than for a server handling network requests.
- **A correctness surface where mistakes are silent.** Interception has to
  get `rename`, `link`, `unlink`, `O_TMPFILE`, `openat` with a dirfd,
  symlinks, hardlinks and mmap writeback all right. One wrong case is a
  build that reads stale content and succeeds — worse than no rollback at
  all.

Snapshots pay at checkpoints instead of continuously, which also matches
the granularity an agent workflow actually needs: work for twenty minutes,
then review. Per-syscall fidelity buys nothing there.

**Where sandlock's approach genuinely wins, and devcroft has no answer.**
It protects the directory you are actually working in. devcroft's rollback
story is per-agent clones, which only helps when the agent is working in a
clone — the fleet case. A single sandbox pointed at your real project
directory has no protection beyond "commit before letting an agent loose".
That is a real gap with a well-understood mitigation, and the mitigation
is why it is not being closed with a syscall-interception layer today. If
"commit first" ever stops being an acceptable answer, this is the entry to
reopen.

### Considered and rejected: port virtualization via `pidfd_getfd`

sandlock gives each sandbox a full virtual port space: the supervisor
intercepts `bind`, performs it on a different *real* port via
`pidfd_getfd`, and filters `/proc/net/tcp` so the child sees only its own.
Multiple sandboxes bind the same number; conflicts resolve transparently;
`sb.ports()` returns the mapping.

More transparent than what devcroft will do — the service genuinely
believes it holds the port, and nothing has to be configured. Rejected on
the same ground as the COW entry above: it needs a seccomp-notify
supervisor outside the sandbox for the sandbox's whole life, which
introduces a component whose death breaks the sandbox's ability to
compute.

devcroft reaches the same *user-visible* outcome — every sandbox uses the
committed port, and the host can still reach a chosen one — by relaying
into the namespace over a unix socket, which is the egress relay run
backwards (`add-port-allocation` design.md P-NEW). It fails better: if the
forwarder dies, ingress stops and the sandbox keeps running.

Worth recording that sandlock's approach is the more elegant one and was
turned down for an operational property rather than a technical one.

### Noted: devcroft's DNS position is stronger, and reading this is how it got checked

sandlock resolves hostnames once at start and pins them in a synthetic
`/etc/hosts`, which constrains what the sandboxed process can resolve.
Checking devcroft against that found something better by accident: a
devcroft sandbox cannot resolve names *at all* — `gethostbyname` raises
`gaierror`, because DNS is UDP and an isolated namespace has no route.
Every name is resolved host-side by the proxy, which then dials the
addresses it just checked.

DNS rebinding needs the client to resolve, and it cannot. `known-gaps.md`
had listed rebinding as an open question; it is now recorded as closed by
construction, with the narrower thing that remains true — same-IP virtual
hosting behind a TLS tunnel the proxy does not inspect — stated in its
place.

### Noted: L7 HTTP rules have a second implementer

sandlock does method/host/path access control through transparent
proxying. `add-backend-capabilities` records the same capability in
`nono-proxy` as `not-adopted` with *"no consumer yet"*. Two independent
implementations make it a real want rather than a hypothetical, which is
worth knowing when devcroft's own proxy is next opened — it already
terminates HTTP and decides by hostname, so method and path are a smaller
step from there than from nothing.

### Noted: a startup-time benchmark

sandlock claims ~5 ms startup against ~200 ms for containers. devcroft has
never measured its own, and the "many environments at near-zero marginal
cost" claim in its README is currently architectural reasoning rather than
a number. A figure to beat, or to be honest about.

---

## nono / nono-cli — <https://github.com/nolabs-ai/nono>

The sandboxing library devcroft depends on (`use-nono-library`), and its
CLI, which devcroft deliberately does not.

Full treatment in `docs/comparison.md`, including why folding devcroft's
opinions upstream is the wrong direction. The capability-by-capability
audit of what the library offers against what devcroft uses is
`add-backend-capabilities` — which has repeatedly turned up things sitting
unused, `IpcMode` above being the latest.

**The CLI's source is public and license-compatible, which changes what
"inspiration" can mean here.** It lives in the same repo under
`crates/nono-cli/` (the published crate is library-only, which is why it
is not in the vendored source, and why this went unread for so long).
Apache-2.0 — and since devcroft is now Apache-2.0 too, its code can be
*adapted with attribution* rather than only studied. That is a different
and cheaper option than the one available while devcroft was MIT.

### Taken: `resource_cgroup.rs`, which is fleet's blocking group already written

`add-linux-agent-fleet` task group 1 (resource control, 0/8) is the 0.2→1.0
roadmap's gate for running N agents — without limits one runaway build
starves the rest. `crates/nono-cli/src/resource_cgroup.rs` is a complete
cgroup v2 implementation of it. Fleet's own task 0 had already identified
that file as where the rendering lives; nobody had opened it.

**It independently reaches D6's conclusion.** Fleet decided to drop manual
delegation as a fallback, on the reasoning that "a hard requirement with a
clear preflight failure is more honest than a fallback that silently
under-enforces". The reference refuses the run at setup when delegation is
unavailable, with no fallback. Two designs arriving at the same call is
worth more than either alone.

Four things it has that fleet's task list did not, now folded in there:

- **The child attaches itself to the leaf right after `fork`**, via an
  inherited `cgroup.procs` fd and only async-signal-safe calls. Without
  it there is a window where the child runs uncapped in the parent's
  cgroup.
- **Controllers are verified present in `cgroup.subtree_control`** and the
  run fails if not — D6's "silently under-enforce" concern as an actual
  check rather than a principle.
- **`memory.high` is deliberately left unset** with swap off, because a
  program over it stalls rather than dying, which presents as a hang.
- **Kernel evidence for why something died**: `memory.events`' `oom_kill`
  and `pids.events`' denied-fork counters turn "the agent vanished" into
  a reason. Silent when zero.

Its stale-leaf sweep — check `/proc/<pid>` before removing a leftover —
is the same rule devcroft already applies to pidfiles via
`state::is_same_process`, arrived at separately.

### Worth reading when the relevant work starts

Not taken, but mapped, so the next person does not start from nothing:

| file | devcroft's corresponding gap |
| --- | --- |
| `terminal_approval.rs`, `approval_runtime.rs` | `add-agent-interaction`'s open question — *who* implements `ApprovalBackend` |
| `rollback_*.rs` (~116 KB) | the rollback gap in `known-gaps.md`; session-based, with a preflight |
| `timeouts.rs` | the wall-clock limit fleet's group 1 now lists |
| `pty_proxy.rs` (~129 KB) | devcroft's own pty handling is a fraction of that; likely edge cases it does not cover |
| `why_runtime.rs` | devcroft's `why`, which answers a narrower question |
| `sandbox_state.rs`, `state_paths.rs`, `session.rs` | direct analogues of devcroft's `StatePaths` and session registry |

### The registry packs, and the one technique in them worth taking

`nono pull <namespace>/<name>` installs a signed pack from
`registry.nono.sh`. Read live rather than inferred — the registry is
public, and `nolabs-ai/claude` v0.1.1 answers on
`/api/v1/packages/nolabs-ai/claude/versions/0.1.1/pull`, sigstore-signed
out of `nolabs-ai/nono-packs` (rekor index 2685283132).

**What a pack contains, measured**: a `policy.json` profile
(`extends`, `groups`, `filesystem`, `network`, `workdir`, `undo`), two
shell hooks, and Claude Code plugin wiring — `.claude-plugin/plugin.json`,
`hooks/hooks.json`, `skills/nono-sandbox/SKILL.md`. **No binaries, no
runtime.** So a pack does not answer `add-agent-workload`'s actual problem,
which is that an agent's runtime is *absent* inside the sandbox
(`node → NO_NODE`); it answers the adjacent one.

It is also not the architecture it first looks like. The plugin's own
description is "teaches Claude Code how to work **inside** a nono security
sandbox" — the same model devcroft has, agent within the boundary, not an
agent on the host whose tool calls are mediated.

**The technique worth taking: tell the agent why it was refused, at the
moment of refusal.** The hook fires on Claude Code's `PostToolUseFailure`
for `Read|Write|Edit|Bash`, gates on a denial signature
(`operation not permitted|EPERM|landlock` — so ordinary failures pass
through untouched), and injects the sandbox's own capability list plus an
instruction: run the `why` equivalent and quote its output verbatim, and
**do not ask the user for permission before diagnosing**.

That gap exists in devcroft today and the expensive half is already built.
An agent that hits a policy denial inside a devcroft sandbox gets a bare
`EPERM`: it does not know it is sandboxed, what it was granted, or that
`devcroft why --path <p> --op <read|write>` will tell it. The likely
failure is a wrong inference — "the file does not exist", "this needs
sudo" — and either a give-up or an absurd request. `why` answers exactly
this question and nothing puts it in front of the thing that needs it.

Taken as a technique, per this file's own rule: the mechanism is a
denial-triggered feedback channel, not this pack, and devcroft's agent
integration is its own to design. Recorded against `add-agent-workload`,
whose three considered options for the runtime did not include reading
what the ecosystem had already solved.

## bubblewrap — <https://github.com/containers/bubblewrap>

Refused as a dependency, read as a reference. `add-mount-isolation` M2
records both halves of that: what refusing it costs (its mount setup
encapsulates real knowledge about what breaks without a complete-enough
`/dev`), and why it is refused anyway (the two standing requirements at
the top of this file).

## ArcBox — <https://github.com/arcboxlabs/arcbox>

A macOS container and VM runtime (Rust, Apache-2.0 + MIT), whose `abctl claude`
puts Claude Code in a disposable Firecracker microVM. Whether devcroft should
follow it into a VM tier is answered in `docs/decisions.md`; what follows is
what was taken instead, per this file's rule — techniques, not tools.

**The layering is the clever part, and it is not the VM.** Firecracker is
Linux/KVM-only, so on macOS it runs *nested* inside a Virtualization.framework
Linux guest — which ArcBox needs anyway for its Docker-compatible engine. They
did not add a VM layer for agents; they reused the one already paid for and put
a cheap VMM inside it. Firecracker rather than a second VZ VM per agent is what
makes N agents affordable: no BIOS, no PCI, virtio only, ~100ms boot. That
arithmetic is fleet's problem too, solved by choosing the VMM rather than by
optimising.

Four things worth having, **none of which needs a VM**:

| Taken | Where it lands |
|---|---|
| `Prepare` — warm pools that spawn a VMM ahead of a boot | devcroft's equivalent cost is provider resolution at `up`, paid per sandbox, every time |
| `Checkpoint` — pause → snapshot → resume, so a booted idle sandbox skips the cold path | no devcroft analogue at all; the closest thing is `up --recreate` doing strictly more work |
| `Adopt`/`discover` — rediscovering a VM that outlived its booter | devcroft solves this with a pidfile plus a health probe. ArcBox validates harder: recorded pid **and** `/proc` candidates, each held to an `--id`/`--api-sock`/jail-root test. A pidfile alone cannot tell a reused pid from the real thing |
| An **id-length budget** derived from `sun_path` | devcroft hit the identical limit (`services::MAX_SOCKET_PATH`, 103 bytes) and turned it into a check that fails at layer `config`. ArcBox turned it into a budget: `/var/jail` is short *on purpose*, because "every byte of the base is a byte AF_UNIX leaves the id" |

The last one is the same discovery arrived at independently, which is the most
useful kind of corroboration — and their handling is the better ergonomics for
the same constraint.

**Two conclusions reached independently, worth recording as confirmation rather
than as ideas:** one sandbox per agent id, surviving TUI exit so `claude
--continue` works because the agent's state lives inside — the shape devcroft's
keeper already has; and a create-time TTL, which `docs/roadmap.md` already
calls "the cheapest bound on an unattended agent" for fleet.

**A third instance of a rule this project keeps rediscovering.**
`arcbox-fc-driver`'s README: *"Nothing above this crate names Firecracker
except the composition root that picks it."* That is the argument for keeping
`SessionBackend` after `remove-gvisor-backend` deleted its only second
implementation, and the argument `decouple-service-supervisor` made for the
supervisor seam. Three independent arrivals at the same conclusion.

**And one thing deliberately not taken: their credential handling is better
than devcroft's plan and should be copied nearly verbatim.** `ANTHROPIC_*` and
`CLAUDE_*` forwarded for that session only, never written into the image or the
sandbox record, every other host variable left behind. Recorded against
`add-agent-workload`'s credentials group. Note also what they could not solve:
*"OAuth credentials from `~/.claude` are deliberately not copied in"* — a
project with a real VM boundary has no answer for subscription auth either,
which is independent evidence for that change's task 7.1 premise rather than
devcroft's own observation about itself.

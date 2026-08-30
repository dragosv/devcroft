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

## bubblewrap — <https://github.com/containers/bubblewrap>

Refused as a dependency, read as a reference. `add-mount-isolation` M2
records both halves of that: what refusing it costs (its mount setup
encapsulates real knowledge about what breaks without a complete-enough
`/dev`), and why it is refused anyway (the two standing requirements at
the top of this file).

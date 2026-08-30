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

### Noted, not taken: copy-on-write without a mount namespace

sandlock stages writes to a working directory in an upper layer via
seccomp interception of filesystem syscalls, committing on clean exit —
*"no mount namespace, no user namespace, no root"*.

This is **not** an alternative to `add-mount-isolation`, which the two are
easy to confuse: that change hides paths so they cannot be named, this one
protects a directory from modification. Different goals, both valid.

It is directly relevant to the agent case, though — an agent that makes a
mess gets rolled back — and there are now two paths to it: this
interception approach, or nono's own `undo` module (content-addressed
snapshots with a Merkle root), which `add-backend-capabilities` records as
unadopted. Whoever wants the capability should compare them rather than
assume the library's.

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

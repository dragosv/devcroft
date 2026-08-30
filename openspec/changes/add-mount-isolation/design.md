# Design — Mount Isolation

## M1 — A mount namespace, not a seccomp filter

**Decision.** Close the AF_UNIX gap by removing the path from the
sandbox's view, not by filtering the syscall that names it.

**Rationale.** Both work. The measured difference is scope and cost:

| | seccomp on `connect()` | mount namespace |
| --- | --- | --- |
| closes | AF_UNIX connect specifically | every path-based reach at once |
| needs | a filter, a policy for it, and a notification loop if anything is to be *allowed* | one `unshare` flag devcroft already passes, plus a mount plan |
| failure mode | a filter that misses a syscall variant | a path that should have been visible and isn't |

The second row is what decides it. devcroft already enters
`unshare(CLONE_NEWUSER | CLONE_NEWNET)` in the keeper's `pre_exec`; this
adds `CLONE_NEWNS` to a call that is already there, and the user namespace
it already creates is what grants the `CAP_SYS_ADMIN` the mount requires.
No new privilege, no helper, no daemon.

The failure modes differ in kind, and the mount one is friendlier: a
missing path fails loudly and immediately at the first access, where a
seccomp filter that misses a variant fails silently and stays wrong.

**Measured before deciding**, not after: masking `/nix` with a tmpfs
inside an unprivileged `unshare(CLONE_NEWUSER | CLONE_NEWNS)` turned a
CONNECTED nix-daemon socket into `No such file or directory`.

Seccomp is not ruled out — `add-egress-proxy`'s D9 wants it for the
proxy-only path regardless, and the two compose. This decision is only
that mount isolation is the right tool for *this* gap.

## M2 — Not bubblewrap

**Decision.** Implement the namespace directly. Do not exec `bwrap`.

**Rationale.** The capability bubblewrap provides is exactly what M1
wants, and its mount setup encapsulates real hard-won knowledge —
merged-`/usr` symlinks, a `/dev` minimal enough to be safe and complete
enough not to break tooling. Refusing it has a genuine cost and this
section should not pretend otherwise: the mount plan is the fiddliest part
of this change, and bwrap has already solved it.

It is refused anyway, because two standing requirements forbid it and
neither is incidental:

> *"the keeper SHALL NOT be executed as a child of a separate sandboxing
> binary"* — `use-nono-library`, lifecycle spec
>
> *"The process tier requires no external backend binary"* — same spec

Those came from removing `nono wrap`, deliberately, to get the
intermediate process out of the tree. A scenario asserts the keeper is a
direct child of whatever started it. Adopting bwrap re-adds precisely what
was removed, and would additionally complicate the fd-inheritance the
whole architecture rests on (`up` binds listeners → keeper inherits →
keeper self-restricts).

**The precedent is `fleet::netns`**: devcroft took the technique
(`unshare`, an ioctl for loopback) rather than shelling out to `ip` or a
helper, for the same reasons. Read bubblewrap's mount setup as a reference
for what a working view needs; do not depend on it.

## M3 — The view must contain the egress proxy socket

**Decision.** The mount plan explicitly includes the sandbox's own proxy
socket path, even though the surrounding state directory is
baseline-denied.

**Rationale, and this is the interaction most likely to be broken by
someone doing the obvious thing.** devcroft's state directory
(`~/.local/share/devcroft`) is denied by the baseline for filesystem
access, so masking it in the mount view looks like a free hardening win.
It is not: it would silently break egress for every isolated sandbox.

The reason is an asymmetry in how three sockets in that directory are
reached:

| socket | how the keeper gets it | survives masking? |
| --- | --- | --- |
| `control.sock` | fd inherited across exec | yes — no path lookup |
| `ssh.sock` | fd inherited across exec | yes — no path lookup |
| `proxy.sock` | `UnixStream::connect(path)`, per connection | **no** |

The first two are bound by `up` before the keeper spawns and passed as
file descriptors, so the path is never resolved again. The proxy socket is
different by necessity — the relay dials it once per outbound connection
(`add-egress-proxy` E7), so its path must resolve inside the namespace for
the sandbox's whole life.

This is the same AF_UNIX property the change exists to constrain, used
deliberately in one place. That is not a contradiction: the point of a
view is that devcroft chooses what is in it. But it means the mount plan
cannot be expressed as "mask the state dir" and must name the exception.

## M4 — Failing closed on an incomplete view

**Decision.** A sandbox whose view cannot be constructed does not start.
It does not fall back to the host's namespace.

**Rationale.** This differs from how network isolation degrades, and the
difference is deliberate. If netns is unavailable, the sandbox falls back
to shared host ports with a warning: the user loses port isolation, which
is a convenience, and nothing they were told is now false.

Mount isolation is load-bearing for a security claim. Falling back would
leave a sandbox that `policy --render` describes as constrained while the
daemon socket, and every other path, remains reachable — the "degraded
capabilities are surfaced, never silent" invariant violated in the worst
direction, since the surfaced warning would be about something the user
cannot act on mid-session.

Hosts that cannot create unprivileged user namespaces already cannot get
network isolation, and `doctor` already reports that. This change extends
the same report rather than adding a second probe.

## Open Questions

1. **What `/nix` actually needs to look like.** The spike masked *all* of
   `/nix`, which closed the socket and would also have removed the
   toolchain — the closure lives in `/nix/store`. The real plan needs
   `/nix/store` read-only and `/nix/var` absent, which the spike did not
   test. Same question for devbox and for a nix-flake provider whose store
   paths differ. **Measure per provider before writing the plan**, since
   guessing here produces a sandbox that starts and then fails at the
   first compile.
2. **`/proc` and PID namespaces.** A private `/proc` is only meaningful
   with `CLONE_NEWPID`, and that makes the keeper PID 1, which must then
   reap. Fleet's D2 already carries this. Decide whether this change takes
   PID isolation too or leaves it to fleet — taking it closes the
   process-visibility gap as well, at the cost of the reaping work.
3. **macOS.** Seatbelt has no mount-namespace equivalent. The gap this
   change closes presumably exists there too, in a different shape, and is
   unmeasured — this project has no macOS host. Whatever ships must degrade
   honestly rather than claiming a boundary it does not hold.

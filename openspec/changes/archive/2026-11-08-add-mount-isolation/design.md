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

**"Merged-`/usr` symlinks" above was read here first and cashed out
literally during task 2.1's implementation, not left as a note to
revisit.** `resolved_grants` canonicalizes every entry, so a grant of
`/lib` (this host: a symlink to `/usr/lib`) only ever bind-mounts the
real target, `/usr/lib` — correct for anything that opens a path through
ordinary resolution, wrong for an ELF binary's own hard-coded interpreter
path. A binary linked on this host names its dynamic linker
`/lib/ld-linux-aarch64.so.1` literally; the kernel's loader resolves that
*inside the view*, where `/lib` does not exist unless something creates
it. Measured: a standalone connect-probe binary — this change's own live
isolation check — failed with a plain `ENOENT` on exec, not a linker
error, until `fleet::mount::setup_merged_usr_compat` started recreating
`/lib`, `/lib64`, `/bin`, `/sbin` as symlinks whenever the host itself has
them. Bubblewrap's own README lists this exact case among its mount
setup's reasons to exist; it was not obvious in advance which concrete
failure it would cause here, only that it would.

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

1. **What `/nix` actually needs to look like — resolved, measured.** The
   spike masked *all* of `/nix`, which closed the socket and would also
   have removed the toolchain — the closure lives in `/nix/store`. Traced
   a real `cargo build` under `strace -f -e trace=file` for all three
   closure-tier providers, using each provider's actual captured
   environment (`flox activate -- env -0` on a derived hook-free copy plus
   the `[hook]` script run separately, matching what `up`/the keeper
   actually does; `nix print-dev-env --json`; `devbox shellenv --pure`) —
   not a bare interactive shell, since that pulls in ambient `PATH`
   entries a real session never has:

   | provider | sample | `/nix/var` touched? |
   | --- | --- | --- |
   | flox | `flox-clap-sample` | no |
   | nix | `nix-flake-sample` | no |
   | devbox | `devbox-citytime-sample` | no |

   Zero occurrences in any of the three traces, including the daemon
   socket path itself. **`/nix/store` read-only and `/nix/var` absent is
   confirmed safe** for a real compile — the plan the spike didn't test.
   The `/nix/var/nix/daemon-socket/socket` and
   `/nix/var/nix/profiles/...` accesses that *do* appear in an untrimmed
   trace belong to `flox activate`'s own `nix build` calls — host-side
   provisioning, already outside any mount view this change constructs.

   **A load-bearing side finding, not this change's job to fix.** Real
   `cargo` builds need `CARGO_HOME` to resolve somewhere writable and
   visible. flox's sample redirects it into the project root via
   `[hook].on-activate` (run inside the sandbox, per
   `fix-provisioning-hooks`), so it stays inside the view for free. The
   nix and devbox providers run **no** hook at all (`nix.rs`,
   `devbox.rs`'s own doc comments), so `cargo` silently falls back to
   `$HOME/.cargo` — outside the project root, outside `/nix/store`, and
   not part of `read_only_grants`. Measured live: a zero-dependency
   `devbox-citytime-sample` build still touches
   `$HOME/.cargo/{.package-cache,.global-cache,config.toml}` for cargo's
   own bookkeeping. This is a pre-existing gap in what the *policy*
   grants for these two providers — independent of mount isolation, whose
   job is to mirror whatever is already granted, not widen it. Worth its
   own follow-up; not tracked further here.

   **A second pre-existing gap, found the same way — running a real `up`
   end to end, not by review.** `flox-clap-sample`'s own `[hook].on-
   activate` (`CARGO_HOME` redirection, `cargo fetch`) fails at session
   time: `lifecycle::hooks::run_one` sends the activation script as
   `SpawnRequest{cmd: "sh", ...}` — a bare, unresolved literal, not
   `DEVCROFT_SHELL` (`src/shell.rs`'s own resolved-from-the-closure
   path, used today only by `ssh/server.rs`). flox-clap-sample's closure
   installs no shell of its own, so `Command::new("sh")` PATH-searches
   into the canonical baseline tail and finds a real host `/bin/sh`.
   **Confirmed pre-existing, not a regression this change introduces**:
   reproduced against the unmodified pre-mount-isolation code (`git
   stash`), where it fails identically but with a different symptom —
   `EACCES` (Landlock denies the host path) there, `ENOENT` (the path
   does not exist in the view at all) here. Same root cause, this
   change only changes which layer refuses it first. Left unfixed here
   for the same reason as the `$HOME/.cargo` gap above: this change's
   job is to mirror what the policy already grants, not to widen the
   set of things that resolve. `tests/mount_view_e2e.rs` and the manual
   end-to-end verification below therefore use `devbox-citytime-sample`
   (no hook at all) rather than `flox-clap-sample`.

   **The real, unmodified `devcroft up` → `status` → `exec` → `down`
   flow was run end to end against `devbox-citytime-sample`, mount
   isolation included** — not only the `__mount_view_probe` harness
   above. `up` succeeded, `status` reported healthy, `exec -- cargo
   build` succeeded inside the running sandbox, and a compiled
   connect-probe run via `exec` confirmed the daemon socket refused
   with `ENOENT` from inside a real session, not just inside the
   probe's own constructed view. This is the authoritative verification
   task 4.4 asks for; the probe-based checks earlier in this section
   established the mechanism works before wiring it into `up` at all.

   Minimal system layer, also measured (union across all three traces,
   excluding failed lookups and PATH-search noise from tools absent from
   a given closure): `/etc/{passwd,nsswitch.conf,ld.so.cache,hosts,
   resolv.conf,host.conf,localtime,gitconfig}`, a CA bundle (one of the
   conventional paths under `/etc/ssl`, `/usr/lib/ssl`,
   `/usr/local/share`), the closure's own libc/dynamic-linker shared
   objects, and `/usr/bin/env` (shebang interpreter). `/proc`:
   `self/{cgroup,exe,maps,statm}` and `sys/vm/overcommit_memory` only —
   no enumeration of other processes. `/dev`: `null`, `tty`, `urandom`,
   `fd/*`.

2. **`/proc` and PID namespaces — resolved: not taken by this change.**
   The measured `/proc` need (above) is five entries, all either
   self-relative or a global sysctl — nothing that requires seeing or
   hiding other processes. A private `/proc` needs `CLONE_NEWPID` only to
   also close process-visibility, which this change's measured
   requirement does not call for. Taking it would make the keeper PID 1
   (reaping becomes mandatory) to satisfy a gap nothing here measures.
   **Leaves PID isolation to fleet's D2**, consuming this change the same
   way fleet already consumes `netns` — the precedent the proposal names.
   The mount view bind-mounts the specific measured `/proc` entries
   rather than the host's `/proc` wholesale, so nothing beyond that
   measured set is exposed either way.
3. **macOS — still open, unmeasured, deferred.** Seatbelt has no
   mount-namespace equivalent and this project has no macOS host. This
   change ships Linux-only, following `fleet::netns`'s own
   `#[cfg(not(target_os = "linux"))]` shape, and `doctor` must report the
   capability as unavailable there rather than silently proceeding
   restricted only by Landlock/Seatbelt's existing tier. Whatever closes
   this later must degrade honestly rather than claim a boundary this
   platform does not hold.

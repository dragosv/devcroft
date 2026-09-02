# Known gaps

The long-form detail behind the README's short "Known gaps" list. Each of
these is a gap in what's actually built, not a design decision —
`docs/decisions.md` has the falsifiable "why not X" reasoning for the
latter.

## Port collisions: fixed

`CompiledPolicy::wants_network_isolation` gives a sandbox its own network
namespace when it declares services or `network.ports`. `devcroft.toml`
being committed is no longer a problem: every git worktree of a repo
declares the *same* port, each sandbox has its own port table, so N of
them binding the identical 5432 no longer collide — no allocation, no
cooperation from the service, no config to write. Verified live:
`tests/network_isolation_e2e.rs` brings up two real sandboxes of one
project, has one hold the port open, and confirms the other binds the
identical number anyway.

**Egress works inside that namespace too**, which was not true when this
section was first written. The original scope excluded any sandbox
wanting `network.allow`, on the reasoning that an isolated namespace has
no route to the host-bound egress proxy and closing that needs a
forwarding helper (pasta/slirp4netns) blocked on `/dev/net/tun`. That
reasoning was about *IP routing*, which devcroft never needed — it needs
TCP streams reaching a proxy. A pathname unix socket crosses a network
namespace, so the proxy gained a unix listener and the keeper relays to
it from inside the namespace. `tests/isolated_egress_e2e.rs` asserts both
properties in one sandbox: it binds its declared port while the host
holds the same number, *and* reaches an allowlisted host while a
non-allowlisted one stays refused.

**A granted port is namespace-local, and that is a behaviour change.**
Before isolation, a dev server bound inside a sandbox answered on the
host's own `127.0.0.1:<port>`; now it does not — measured, both
directions. That is the same property that stops two sandboxes colliding,
seen from the other side. `up` prints a note naming the
`ssh -L <local>:127.0.0.1:<port> <name>.devcroft` forwarding when it
applies, `policy --render` marks the ports
namespace-local, and `tests/host_port_reachability.rs` asserts both
directions so a future host-side mapping (fleet's D8) has to update the
claim rather than silently contradict it.

Two further limits worth knowing:

- The relay binds the proxy's own port number inside the namespace, which
  is what keeps `HTTP_PROXY` and the compiled `proxy_only` gate identical
  isolated or not. If a manifest also declares that number in
  `network.ports`, isolation is skipped with a warning rather than
  breaking egress — the proxy port is OS-assigned from the ephemeral
  range, so this is rare rather than theoretical.
- A host that cannot create unprivileged network namespaces degrades to
  the shared host port table, with one warning at `up`.

Fleet (`add-linux-agent-fleet`) is a second, harder consumer of the same
primitive — N agents under one supervisor, plus an optional host-side
mapping for reaching one from outside — not yet built.

## UDP was not blocked by `network.default = "deny"` — fixed by isolation

**Landlock's network rules are TCP-only.** `NetPort` gates
`connect`/`bind` for AF_INET *stream* sockets and says nothing about
datagrams, so a sandbox with `network.default = "deny"` and an allowlist
naming one host completed a full DNS round-trip to `8.8.8.8:53` — query
sent, 61 bytes of reply received. The allowlist constrained nothing
because it was never in the path. DNS exfiltration and QUIC/HTTP3 would
both have bypassed the proxy entirely.

nono *does* ship a seccomp filter denying UDP, raw and non-stream sockets.
It is `apply_auto`'s fallback for pre-V4 Landlock kernels and is never
installed on a V6 host — the same trap `add-egress-proxy` task 0 hit with
`install_seccomp_proxy_filter`. Reading the library for "does it deny
UDP" answers yes, about the wrong path.

**Closed** by giving every `network_block` sandbox its own network
namespace, not only those declaring ports. An isolated sandbox has no
route out at all, so UDP fails with `ENETUNREACH` whatever the policy
layer covers. Egress that is wanted still works, through the relay.
Asserted in `tests/udp_egress_denied.rs`, which was verified to fail
(`LEAK 61`) with the fix reverted.

**Residual:** a host that cannot create unprivileged network namespaces
gets no isolation and therefore still leaks UDP. `up` warns about the
missing namespace, but the warning is about port collisions and does not
mention this. Closing that properly needs the seccomp filter nono only
installs on old kernels — an upstream ask, or a devcroft-side filter.

## Unix sockets are not mediated by Landlock — both halves now closed

**Landlock's network rules cover TCP only.** `connect()` to a pathname
unix socket falls through to ordinary filesystem permissions, so
Landlock alone never mediated it — *including sockets in directories the
compiled policy does not grant*. That was measured, not inferred, by
`tests/unix_socket_not_mediated.rs`, and it is the reason this entry
existed.

**`add-mount-isolation` closes the pathname half, on Linux.** Every
sandbox now gets its own mount namespace and filesystem view
(`fleet::mount::construct_view`), and a socket outside that view simply
does not resolve — `connect()` fails with `ENOENT`, not a permission
error, because there is no path left to name. Measured, not assumed:
`tests/unix_socket_not_mediated.rs` (same file, inverted) now asserts
the refusal, live, including the instance that mattered most —
`/nix/var/nix/daemon-socket/socket`, `srw-rw-rw-` under nix's multi-user
model, previously reachable with `/nix` ungranted and now not. That was
exactly the package-manager authority `sandbox-provisioning` P2a/P2b
says an agent must not hold; that guarantee is now kernel-enforced
rather than resting on devcroft's own refusal to grant a path.

**Still open on macOS.** Seatbelt has no mount-namespace primitive, so
the identical ungranted probe connects there unchanged (measured, macOS
15). Worth stating precisely, because the first macOS run said the
opposite — probing `/tmp/<dir>/p.sock` there is refused with `Operation
not permitted`, but only because `/tmp` is a symlink to `/private/tmp`
and Seatbelt denies the ungranted symlink traversal; the *same socket*
named `/private/tmp/<dir>/p.sock` connects. Symlink traversal being
mediated is not AF_UNIX being mediated, and the test now canonicalizes
so the difference cannot be misread again. A mount namespace is a Linux
primitive; closing this on macOS needs a different mechanism, not a port
of this one.

**The gap had two halves; only the harder one needed new machinery, and
the other turned out to already be closed.** Unix sockets come in two
kinds and Landlock treats them differently:

- **Abstract** sockets (`@`-prefixed, no filesystem path — dbus, X11,
  PipeWire, systemd-journald on a typical desktop) *are* expressible, and
  **this half was already closed, not open** — corrected here after
  `add-backend-capabilities` traced it properly. Landlock V6 has
  `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET`; nono requests it whenever
  `IpcMode::SharedMemoryOnly` is set, and that is nono's own `#[default]`
  — devcroft never calls `set_ipc_mode` at all, so every sandbox already
  gets it, with no code change ever needed. A prior version of this entry
  read "devcroft still does not set it", reasoning from "we never call
  `set_ipc_mode`" without checking what the *unset* default resolves to.
  Measured live, not just traced: a probe (`__abstract_socket_probe`)
  applying devcroft's real, unmodified `CapabilitySet` against a real
  abstract socket gets `EPERM`. Unrelated to the mount view either way —
  a namespace does not change which abstract sockets are visible;
  Landlock's own scope rule is the only lever, and it was already pulled.
  Requires Landlock ABI V6+; older kernels get no enforcement here, which
  `add-backend-capabilities`'s matrix records as the platform boundary
  rather than a gap in what devcroft asks for.
- **Pathname** sockets (`/nix/var/nix/daemon-socket/socket`) were not
  expressible at any Landlock ABI — the half `add-mount-isolation`
  closes, described above.

**How it was closed, for the record.** Not a Landlock rule (none exists
for AF_UNIX) and not seccomp filtering on `connect()` — a mount
namespace, per sandbox, whose view is narrow enough to remove the socket
and wide enough that a real compile still works
(`/nix/store` read-only, `/nix/var` absent; measured across flox, nix,
and devbox, zero `/nix/var` accesses in any real build). Seccomp on
`connect()` remains a live idea for `add-egress-proxy`'s D9
proxy-only path, independent of this.

**The same property is load-bearing in the other direction**, which is
why it was worth closing this way rather than only patching the one
instance: a pathname unix socket crosses a *network namespace* too. That
is what lets a network-isolated sandbox reach devcroft's host-side
egress proxy without a TUN device or a forwarding helper — and it is
exactly why the mount view has to name that one socket back in
explicitly (design.md M3) rather than only ever removing paths.

## Exec is not mediated on macOS, so host binaries run inside the sandbox

**Measured on macOS 15.** `own-policy-baseline`'s result — a build works
from the closure while `/usr/bin/gcc` and `/bin/ls` are denied — is a
*Linux* result. Landlock treats execution as a filesystem right
(`FS_EXECUTE`), so a binary under no granted path is refused. Seatbelt does
not: nono's macOS profile emits an unconditional `(allow process-exec*)`
(nono 0.74.0, `sandbox/macos.rs`), and every host binary therefore runs
inside a macOS sandbox regardless of the compiled policy.

Reads are still mediated, which is what makes this easy to miss: inside a
real sandbox on this host, `ls -l /usr/bin/gcc` is `Operation not
permitted` while `/bin/ls` runs and lists the project directory. A path
can be unreadable and executable at the same time.

Two consequences worth stating plainly:

- The closure-tier guarantee — "the toolchain your build uses comes from
  the closure, not the host" — is enforced on Linux and **cooperative on
  macOS**: a process that names an absolute host path gets the host's
  binary. What still holds on macOS is that the *environment* is the
  closure's, so anything resolving through `PATH` gets closure tooling.
- `tests/devbox_provider_e2e.rs`'s host-toolchain denial probes exec
  mediation first and self-skips where there is none, rather than
  asserting a property the host cannot provide. It starts asserting again
  by itself if that changes.

This is nono's decision, not devcroft's, and closing it means either a
narrower `process-exec` rule upstream or accepting that the macOS tier
protects against accidents rather than absolute paths — which is what
`docs/threat-model.md` already says the tier is for. **`doctor` now says so
out loud** on macOS — a `[WARN]` line under the backend's own verdict,
rather than leaving it to this file. Not an `up` warning: `up`'s warnings
are all "this manifest asked for something this host cannot enforce", and
exec mediation is asked for by nothing and avoidable by no one, so warning
there would fire on every run of every project about something no manifest
can change.

## macOS grants are per-spelling, not per-directory

**Measured on macOS 15.** Seatbelt matches paths as written. `/tmp` and
`/var` are symlinks (`/private/tmp`, `/private/var`), so a grant issued for
one spelling of a directory does not necessarily cover the other, and a
`$TMPDIR` path is affected by default. Landlock has no equivalent problem —
it works on inodes, so every spelling of a directory is the same object.

Two distinct symptoms, both traced to it:

- A sandbox whose project root was given as `/var/folders/…/p` could not
  spawn a hook with that cwd at all (`Operation not permitted`), while the
  identical spawn under `/private/var/folders/…/p` succeeded.
- A hook naming an absolute path through the symlinked spelling is denied
  even when its own project root is granted.

`up` now resolves the project root once, so the compiled grant,
`Meta.project_root`, and the cwd of every session and hook agree on the
spelling the backend sees. The CLI never had the first symptom —
`current_dir()` returns the resolved path already — which is why this
surfaced only through library callers (the test suite). The second is
inherent: a path that a project *writes down* in the symlinked form is not
the path the policy granted. Prefer real paths in manifests and hooks on
macOS.

The library devcroft builds on emits both spellings when it is handed the
symlinked one (nono 0.74.0, `sandbox/macos.rs`), which is a real mitigation
but only for the spelling that reaches it; devcroft resolving first means
it reaches the backend in the form the kernel will compare against.

## `network.ports` is not a bind limit on macOS

Landlock scopes bind by port (`NetPort`), so a sandbox that declares
`ports = [5432]` can bind 5432 and nothing else. Seatbelt has no
port-scoped bind rule at all: nono emits a blanket `(allow network-bind)`
whenever any local port is wanted, and a process can then bind whatever it
likes. Measured — a manifest granting one port, a process binding a
different one, no error.

The declared ports still work, so the manifest is not *wrong* on macOS,
only weaker than it reads: treat the list as intent rather than a limit
there. `up` warns whenever a manifest declares ports on a host that cannot
scope them — this one *is* manifest-requested, which is what makes it an
`up` warning rather than a `doctor` line. `tests/network_ports_listen.rs`
asks the same detection before asserting the ungranted half, so it starts
asserting again by itself if a backend gains the capability.

## Two macOS grants that are broader than they look

Both are visible in `policy --render` rather than hidden, and both exist
because Seatbelt's model differs from Landlock's in ways no manifest can
express.

- **`/dev`, granted read-write on macOS** (`baseline`). `openpty(3)` opens
  `/dev/ptmx` *and* the slave it returns, `/dev/ttysNNN`, whose number is
  unknowable before the call; nono's path API has no globs and the node
  does not exist until the kernel makes it. With only `/dev/ptmx` granted
  every pty session fails — `devcroft shell`, and the SSH shell channel
  editors use, both died with "keeper refused to spawn: Operation not
  permitted". Linux keeps the narrow `/dev/pts` grant. The narrower fix is
  upstream: nono already special-cases `^/dev/ttys[0-9]+$` for `file-ioctl`
  and emits `(allow pseudo-tty)`, so it needs the matching read/write rule.
- **The services supervisor socket, granted `ConnectBind`**
  (`provider:<name>`, only when services are declared). Seatbelt's
  `(deny network*)` covers AF_UNIX bind, so process-compose could not bind
  its own supervisor socket inside the sandbox and every macOS sandbox with
  services came up supervising nothing. Landlock cannot express AF_UNIX at
  all, which is why the grant is inert on Linux — and why this went
  unnoticed until the suite ran on a Mac.

## No inter-sandbox process visibility separation

Landlock hides nothing: sandboxes share the host's raw process namespace.
Fixed by `add-linux-agent-fleet`'s per-agent PID namespaces, not yet built.

What this means in practice turned out narrower than originally assumed,
though. On a Landlock **ABI V6** host (`doctor` reports the ABI level; this
repo's own devcontainer is V6), `tests/process_tier_landlock_boundaries.rs`
proves live that a sandboxed process can neither `kill()` nor read
`/proc/<pid>/*` for a process outside its own sandbox — V6's signal-scoping
LSM hook and the default-deny filesystem policy (which covers `/proc` like
any other ungranted path) close both, even with no PID namespace to enforce
it structurally. This is kernel-version-dependent, not a blanket guarantee:
older kernels without ABI V6 would plausibly still allow it, and `doctor`'s
ABI line is how to know which regime a given host is in.

## Domain filtering: enforced on Linux, unverified on macOS

`add-egress-proxy` shipped a real, enforced domain filter on Linux —
Landlock `NetPort` gates every `connect()` except to a resident, per-session
-authenticated proxy, which decides by hostname. `docs/decisions.md`'s
older framing, that domain filtering everywhere was merely cooperative, no
longer describes Linux.

Whether macOS Seatbelt enforces the equivalent `NetworkMode::ProxyOnly`
gate as strictly, or only adds a permissive rule without narrowing anything
else, is **unverified** — the pinned library's own doc comment for the
macOS output reads as a scoped allow rule, which would argue for "enforced"
under Seatbelt's default-deny model, but that specific path has still not
been measured live, and this project does not ship a security claim it
hasn't measured. (The section above *is* a macOS measurement, so the
blocker is now the measurement itself, not the absence of a host.)
The degraded-on-macOS warning stays on until someone can check.

On Linux, the original assumption was that a process could always bypass a
domain allowlist with a raw socket straight to an unresolved IP.
`tests/process_tier_landlock_boundaries.rs` tested that directly and found
it doesn't hold: `policy --render` shows `network.block: true` even with an
allowlist set, and a raw socket to an IP unrelated to any allowed domain
gets a kernel-level `Permission denied` — nono's own Landlock network
scoping, not an unenforced proxy hint the socket simply never talks to.
**DNS rebinding is closed, and by construction rather than by care.** A
sandboxed process cannot resolve names at all — measured: `gethostbyname`
inside a sandbox raises `gaierror`, because DNS is UDP and the sandbox's
network namespace has no route out. Every name is resolved host-side, by
the proxy, which then dials the addresses it just checked rather than
resolving a second time. Rebinding needs the *client* to resolve, and it
cannot. This is a stronger position than pinning a synthetic `/etc/hosts`
(what `sandlock` does), which constrains the client's resolution rather
than removing it.

**Same-IP virtual hosting is still open**, and it is the narrower thing
that remains true. If an allowed name and a disallowed one share an
address, a client that asks for the allowed name gets a TLS tunnel to
that address and can send whatever `Host:` header it likes inside it. The
proxy decides by the name in `CONNECT` and does not inspect the tunnel —
TLS interception is an explicit non-goal. Untested, and not claimed as
safe.

## An agent working in your real directory cannot be rolled back

devcroft confines *where* a sandbox can write — the project root and
little else. It does not version what happens inside that root. An agent
that deletes the wrong files, or refactors 200 of them badly, has done so
to your actual working tree, and devcroft offers no undo.

The mitigation is the obvious one and it does work: commit before letting
an agent loose. It is recorded as a gap anyway because the tool's own
pitch is unattended agents, and "remember to commit first" is exactly the
kind of instruction that fails at the moment it matters.

Two mechanisms would close it, neither built:

- **Per-agent git clones** (`add-linux-agent-fleet` D7, and implemented on
  the `fleet/workspace-isolation` branch). Rollback by discarding the
  clone. Covers the fleet case completely and the single-sandbox case not
  at all, since there you are deliberately working in the real directory.
- **Snapshots** — fleet task 34 names nono's content-addressed `undo`
  module as the candidate, deferred there as "an optimisation on top" of
  clones. This is the one that would cover working in place.

A third approach exists and was considered and rejected: copy-on-write via
seccomp interception of filesystem syscalls, as `sandlock` does. See
[prior-art.md](prior-art.md) for why — briefly, it requires a supervisor
outside the sandbox whose death would break the sandbox's ability to
compute, which devcroft currently has no component of.

## No cgroup resource limits

A runaway build in one sandbox can affect the whole host — nothing today
caps CPU or memory per sandbox. Planned: cgroup v2 scope units per keeper
on Linux; no macOS equivalent exists. Also fleet's subject
(`add-linux-agent-fleet` task group 1).

## Provisioning runs on the host — with one exception now closed

Resolving a provider environment happens before any boundary exists. For
flox, whose `[hook].on-activate` is arbitrary project shell, devcroft now
materializes from a derived hook-free copy of the environment and runs the
hook *inside* the sandbox instead, so no project code executes unconfined.
The rest of provisioning still runs host-side; `sandbox-provisioning` is
the change that moves it. An upstream request that would make the flox
split a supported contract rather than devcroft's inference is drafted at
[flox-confined-activation-issue.md](flox-confined-activation-issue.md).

The nix provider does not have even the historical version of this gap: it
reads the dev shell's environment as structured data and never evaluates
the `shellHook`.

## A `filesystem.allow` grant for a nonexistent path is silently dropped

`policy --render` still shows it as granted, with its `manifest:` origin,
but the backend ignores grants whose target is missing when the profile is
applied — so the rendered policy is not the policy in force. This is the
one gap that contradicts a stated invariant ("deterministic and
inspectable", "degraded capabilities are surfaced, never silent") rather
than just missing a feature. Create the directory before `up` as a
workaround. Found during task 6.5.

## Zed's remote server connects and transfers but does not start

Its forked daemon exits without logging; not yet attributed to devcroft.
Zed also needs five separate `$HOME` grants, one of which is the local
editor's own data directory. VS Code and Cursor are unaffected. Full
detail: [ssh-validation.md](ssh-validation.md).

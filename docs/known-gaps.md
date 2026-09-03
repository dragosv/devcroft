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
seen from the other side. `up` prints a note naming
`devcroft ssh -L` when it applies, `policy --render` marks the ports
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

## Unix sockets are not mediated by Landlock — both halves now closed, on both platforms

**Landlock's network rules cover TCP only.** `connect()` to a pathname
unix socket falls through to ordinary filesystem permissions, so
Landlock alone never mediated it — *including sockets in directories the
compiled policy does not grant*. That was measured, not inferred, by
`tests/unix_socket_not_mediated.rs`, and it is the reason this entry
existed.

**`add-mount-isolation` closes the pathname half.** Every sandbox now
gets its own mount namespace and filesystem view
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

### macOS closes the same gap on the other axis (`add-macos-unix-socket-scoping`)

Everything above is Linux. macOS has no user/mount namespace, so for a
while this entry recorded the pathname half as closed on Linux and simply
open on macOS. **It is now closed there too, and it needed no new
mechanism — only a measurement.**

Seatbelt classifies a unix-socket `connect()` as `network-outbound`, not
as filesystem access. So the `(deny network*)` rule devcroft *already*
compiles for `network.default = "deny"` covers AF_UNIX, and always did.
Measured live on macOS 15.7.4 (arm64) against this host's real
`nix-daemon` socket, the same instance that mattered on Linux:

- Network unrestricted: the sandbox connects to the daemon socket **even
  though `stat()` on the same path is denied**. `connect()` is not gated
  by the filesystem layer at all — that is the gap, reproduced.
- `network.default = "deny"`: the same connect is refused `EPERM`, with
  no devcroft code change. The mechanism was shipping; nobody had checked
  it, so nothing claimed it.
- Reachable again only via an explicit `nono::UnixSocketCapability` grant
  for that exact path, which is what keeps a sandbox's own egress
  reachable and admits no other socket.

Asserted in `tests/unix_socket_not_mediated.rs`'s macOS half, which is
split from the Linux half deliberately: the two platforms produce
different failure shapes (`EPERM` from a rule, versus `ENOENT` from a
path that no longer exists), and one shared assertion would be vacuous on
whichever platform it was not written for.

**Three residual limits, all narrower than the gap was but none of them
nothing:**

- **It is scoped to deny-default sandboxes.** An `allow`-default macOS
  sandbox still reaches any world-accessible unix socket, where an
  `allow`-default *Linux* sandbox does not — a mount view removes the path
  regardless of network mode. Same manifest, weaker guarantee on macOS.
- **Reachability only, not visibility.** macOS gets no filesystem view, so
  nothing narrows what a sandbox can *see*; only what it can dial. The
  broader `add-mount-isolation` claim stays Linux-only, honestly.
- **A `filesystem.allow` grant does not open a socket on macOS, and does
  on Linux.** The two layers are orthogonal in the backend library
  (measured: granting the socket's path makes `stat()` succeed and leaves
  `connect()` refused), whereas a Linux mount view includes granted paths
  and so admits the socket. Nothing in devcroft's manifest surface can
  express a macOS unix-socket grant today; the only one compiled is the
  sandbox's own egress path.

`devcroft doctor` reports this as `pathname-unix-sockets:
enforced (degraded)` on macOS, with both degradations named in the entry
itself rather than in a footnote.

## Host binaries execute on macOS, even at ungranted paths

**`own-policy-baseline`'s "the host toolchain is denied" property is Linux-only,
and nothing said so until now.** It was measured on Linux, where Landlock's
default-deny filesystem policy covers execution like any other access. macOS does
not work that way: the backend library's Seatbelt profile carries an
unconditional `(allow process-exec*)`, and Seatbelt treats executing a file as a
separate operation from reading it.

Measured live in a real devcroft sandbox on macOS 15.7.4, with a devbox closure
and `network.default = "deny"`:

- `/bin/echo`, `/bin/ls`, `/usr/bin/gcc` and `/usr/bin/clang` all **execute**,
  none of them granted by the manifest, the provider, or the baseline.
- Reading those same paths is **refused**: `ls -l /usr/bin/gcc` from inside the
  same sandbox gets `Operation not permitted`, as does `cat /etc/hosts`,
  `ls ~/.ssh`, and `ls /usr/bin`.

So the filesystem boundary is real and enforced; it simply does not extend to
`execve`. The practical consequence is narrower than it first looks — a host
binary that runs still cannot *read* anything the policy denies, so it cannot
exfiltrate project files it was not already granted — but "the sandbox runs only
what the closure provides" is not true on macOS, and a build that silently picks
up a host tool will succeed there and fail on Linux.

`tests/devbox_provider_e2e.rs` asserts the denial on Linux and is gated off on
macOS with a pointer here, rather than the assertion being deleted or weakened to
pass on both.

Not yet investigated: whether `nono` can be asked to scope `process-exec*`, or
whether devcroft should refuse to claim closure-tier semantics on macOS until it
can. Both belong to `own-policy-baseline`, which is where the property was
established.

## A C toolchain from a closure cannot link on macOS

Related to the above but a separate cause, and it makes the devbox/nix closure
tier genuinely less useful on macOS rather than merely less strict.

A C compile from a nix closure fails **at link**, not at compile: the
`cctools-binutils-darwin` linker wrapper drives `ld` through a shell process
substitution and reads `/dev/fd/63`, which devcroft's macOS baseline does not
grant. The baseline grants `/dev/ptmx`, `/dev/null` and `/dev/urandom` and
nothing else under `/dev`, so the read is refused and the build stops with
`collect2: error: ld returned 1 exit status`.

Granting `/dev/fd` would fix it, and that is a real baseline widening rather than
an oversight to patch quietly — `/dev/fd` exposes every file descriptor the
process holds, which is exactly the kind of grant `own-policy-baseline` exists to
make deliberately and with a measurement behind it. Left to that change.

Found while making the test suite run on macOS for the first time
(`add-macos-unix-socket-scoping`); the Linux half of the same test is unchanged
and still asserts the full compile-link-run path.

## Interactive pty sessions are refused on macOS

**`devcroft shell` does not work on macOS, and neither does an SSH session that
asks for a pty.** Both fail with `keeper refused to spawn: Operation not
permitted`. Non-pty sessions (`devcroft exec`) are unaffected and work normally.

The cause, measured from inside a real sandbox rather than inferred from the
symptom: `openpty()` allocates a master and then `open()`s the corresponding
**slave** (`/dev/ttysNNN`). The compiled profile grants the master — `/dev/ptmx`
is a baseline `filesystem.allow` entry — and the backend library adds tty
*ioctl* rules for slave paths, but nothing grants read or write on the slave
itself. Directly confirmed in a running sandbox: `/dev/ptmx` is readable, and
opening `/dev/ttys000` is refused.

devcroft cannot close this on its own. Baseline grants are literal paths and the
slave path is allocated per session, so expressing it needs a pattern rule; the
backend library already emits regex-based tty rules on macOS (for `file-ioctl`)
and would need to extend them to read/write. Granting all of `/dev` instead
would be a far larger widening than the problem warrants. This is an upstream
ask, in the same category as the one already drafted about gating the library's
trust module.

`tests/shell_up.rs` skips on macOS naming this entry, rather than asserting
something weaker — so closing the gap makes the test run again by itself.

## `network.ports` is all-or-nothing on macOS

`network.ports` is documented as an allowlist: the sandbox may listen on the
ports it names and no others. That holds on Linux, where Landlock's `NetPort`
scopes `bind` per port. **It does not hold on macOS**, and the capability matrix
claimed it did while carrying its own note that nobody had checked.

Seatbelt has no per-port form of `network-bind`. Granting any port therefore
emits a blanket `(allow network-bind)` / `(allow network-inbound)` — the backend
library's own source says so in as many words — and every other port becomes
bindable too. Measured: with `ports = [X]` declared and `network.default =
"deny"`, a probe bound an *ungranted* port successfully.

The outbound half is unaffected and does hold: a deny-default sandbox on macOS
still cannot connect out to an ungranted destination (asserted by the same test,
not gated).

`up` now warns when a manifest asks for this on a host that cannot deliver it —
`network.ports (per-port listen scoping)` — and `devcroft doctor` reports
`network-block-and-ports` as `enforced (degraded)` on macOS with the reason
named. Closing it properly would need something Seatbelt does not offer; the
honest position is the declared degradation.

## A grant does not cover the symlinked spelling of its own path on macOS

devcroft canonicalizes every filesystem grant before handing it to the backend,
so the compiled policy names `/private/tmp/proj` where the manifest (or the
shell, or `$TMPDIR`) says `/tmp/proj`. On Linux that is invisible, because the
paths involved are not symlinks. On macOS `/tmp` → `/private/tmp` and `/var` →
`/private/var` both are, and the sandbox denies the un-canonicalized spelling of
a path it has granted.

Measured, in a sandbox whose project root was granted normally:

```
touch /var/folders/…/T/proj/FILE      -> Operation not permitted
touch /private/var/folders/…/T/proj/FILE -> OK
touch FILE                             -> OK   (relative, from the project root)
```

Relative paths and canonical absolute paths both work, so this mostly bites
projects living under `/tmp` or `$TMPDIR` — which is rare for real projects and
common for test fixtures and generated scratch directories. A flox
`[hook].on-activate` writing to `$TMPDIR/...` is the case that actually surfaced
it.

The backend library already does dual-path emission for unix-socket grants
(emitting both `original` and `resolved` so `/tmp/x.sock` and
`/private/tmp/x.sock` both match); filesystem grants emit only the resolved form.
Closing this means emitting both there too — devcroft-side when it builds the
grant list, or upstream. Not attempted here; it belongs with
`own-policy-baseline`, which owns what the compiled grant set contains.

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
under Seatbelt's default-deny model, but this project has no macOS host to
measure it live on, and does not ship a security claim it hasn't measured.
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

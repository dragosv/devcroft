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

## Unix sockets are not mediated by the policy

**Landlock's network rules cover TCP only.** `connect()` to a pathname
unix socket falls through to ordinary filesystem permissions, so a
sandboxed process reaches any unix socket whose DAC allows it —
*including sockets in directories the compiled policy does not grant*.
Measured, not inferred: `tests/unix_socket_not_mediated.rs` runs a real
Landlock-restricted process with only its cwd granted and connects to a
socket under `/tmp` regardless.

The instance that matters: `/nix/var/nix/daemon-socket/socket` is
`srw-rw-rw-` under nix's multi-user model, and a sandbox connects to it
with `/nix` ungranted. That hands the sandbox whatever authority the nix
daemon grants an unprivileged client — realizing store paths, building
derivations — which is exactly the package-manager authority
`sandbox-provisioning` P2a/P2b says an agent must not hold. That change's
design.md previously stated a hook "does not silently receive a writable
`/nix` or the daemon socket"; the second half of that was not true, and
is now corrected there.

Bounded, but real. The daemon enforces its own protocol and nix
deliberately makes that socket world-accessible, so this is not arbitrary
host access — it is the authority nix itself extends to any local user.
The same is not true of every socket: a Docker socket reachable this way
would be a full host compromise, and devcroft's policy would not stop it.

**The gap has two halves, and only one needs new machinery.** Unix
sockets come in two kinds and Landlock treats them differently:

- **Abstract** sockets (`@`-prefixed, no filesystem path — dbus, X11,
  PipeWire, systemd-journald on a typical desktop) *are* expressible.
  Landlock V6 has `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET`, nono requests it
  when `IpcMode::SharedMemoryOnly` is set, and **devcroft never sets it**
  — so this half is open for no reason beyond nobody having connected the
  two. One method call. This devcontainer has no abstract sockets at all
  (`ss -xl` shows zero), so it is unmeasurable here and real on a normal
  desktop.
- **Pathname** sockets (`/nix/var/nix/daemon-socket/socket`) are *not*
  expressible at any Landlock ABI. This is the half the nix-daemon finding
  is about, and the half that needs the mechanism below.

**Closing the pathname half needs a mount namespace, not a Landlock
rule.** No Landlock
ABI expresses AF_UNIX at all, so the fix has to come from somewhere else.
Two candidates, and the cheaper one is better: seccomp filtering on
`connect()` (the machinery `add-egress-proxy`'s D9 contemplates for the
proxy-only path), or simply not having the path in the sandbox's mount
view. Measured: masking `/nix` with a tmpfs inside an unprivileged
`unshare(CLONE_NEWUSER | CLONE_NEWNS)` turns the connect into `No such
file or directory` — nothing to filter, because there is nothing to
name. That is `add-linux-agent-fleet` task group 2's mount plan, which
already exists as a task for other reasons.

This entry originally said seccomp was what it needed. That was one
answer stated as the only one.

**Specified as `add-mount-isolation`**, which also carries the harder half
this entry does not: a view narrow enough to close the gap and wide enough
that a real compile still works. The spike that proved the mechanism
masked all of `/nix`, which would have removed the toolchain along with
the socket — `/nix/store` has to stay.

**The same property is load-bearing in the other direction**, which is
why it is worth understanding rather than only patching: a pathname unix
socket crosses a *network namespace* too. That is what lets a
network-isolated sandbox reach devcroft's host-side egress proxy without
a TUN device or a forwarding helper. One mechanism, one wanted
consequence and one unwanted one.

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

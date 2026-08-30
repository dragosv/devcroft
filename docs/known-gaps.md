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

Two limits worth knowing:

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

**Closing it needs a mount namespace, not a Landlock rule.** No Landlock
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
Left genuinely open (untested, not claimed as safe): whether the *allowed*
domain's own resolved-IP scope is wider than intended — a different service
on the same allowed IP, or DNS-rebinding-shaped tricks.

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

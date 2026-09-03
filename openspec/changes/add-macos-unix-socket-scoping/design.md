# Design — macOS Unix Socket Scoping

## Context

`add-mount-isolation` closed the pathname-AF_UNIX gap on Linux with a per-sandbox mount
namespace (`fleet::mount::construct_view`). That mechanism does not exist on macOS —
Seatbelt has no user/mount namespace equivalent — and an attempt to make mount
isolation unconditional across platforms briefly broke every macOS `up` before being
corrected to degrade gracefully (warn, proceed Seatbelt-only), which is where this
change starts from: macOS today is exactly as exposed to this gap as it was before
`add-mount-isolation` existed.

**This document was written before anything had been run, and has since been
measured.** Every finding below started as a reading of `nono` 0.74.0's own macOS
sandbox source (`src/sandbox/macos.rs`) — an argument, not a measurement. Open
Question 1 has now been answered live on **macOS 15.7.4 (arm64)**, against that
host's own real `nix-daemon` socket, and the Decisions below carry the measured
result inline. The premise that "this repository has no macOS host" was simply
false; the spike was runnable all along, and running it changed one of the two
decisions materially (S2).

## Goals / Non-Goals

**Goals:**
- Close the pathname-AF_UNIX gap on macOS, at the same strength `filesystem-view`
  closes it on Linux — an ungranted socket is unreachable, the sandbox's own proxy
  socket is not.
- Do it through Seatbelt's own network-outbound classification, since that is the
  mechanism macOS actually has, not a port of the Linux mount view.

**Non-Goals:**
- **Not a macOS filesystem view.** No user/mount namespace primitive exists on macOS
  and none is proposed. The broader `filesystem-view` claim — narrowing what a sandbox
  can *see*, not just what it can reach over the network — stays Linux-only, honestly.
- **Not a claim before a measurement.** See Open Question 1. This document argues a
  mechanism; it does not assert one holds.
- **Not a change to `network.allow`'s own domain-filtering semantics.** This is about
  the deny-default baseline reaching AF_UNIX, not about widening or narrowing what a
  hostname allowlist covers.

## Decisions

## S1 — Close the gap on the network axis, not by inventing a filesystem one

**Decision.** Use `network.default = "deny"` (already compiled to a Seatbelt
`(deny network*)` rule) as the mechanism, rather than looking for or building a
macOS-side filesystem-view equivalent.

**Rationale.** Read directly from the pinned library's own macOS sandbox generation
(`nono::sandbox::macos`): Seatbelt classifies a `connect()` to a unix-domain socket
*by path* as `network-outbound`, not as filesystem access — there is no separate
Seatbelt filter category for AF_UNIX the way Landlock's Fs/Net split has one. A rule
that denies `network*` therefore denies unix-socket connects the same way it denies
TCP ones, with no additional configuration.

The evidence this is real and not an assumption about undocumented behavior: the
library's own emitted profile grants `mDNSResponder` — a real, well-known unix-domain
socket — explicitly, via `(allow network-outbound (path "/private/var/run/
mDNSResponder"))` (and the `/var/run/...` compatibility path alongside it). That grant
is only necessary at all if the surrounding `(deny network*)` already reaches unix
sockets; a library that didn't believe its own deny rule covered AF_UNIX would have no
reason to carve out an exception for one.

**MEASURED — confirmed, and the emitted profile matches the source reading exactly.**
Run live on macOS 15.7.4 (arm64), applying a real `CapabilitySet` through
`nono::Sandbox::apply_auto`, with the host's own `nix-daemon` socket
(`/nix/var/nix/daemon-socket/socket`, `srw-rw-rw-`) as the target and a control
connect from outside the sandbox proving it live first:

| network mode | ungranted socket | with a `UnixSocketCapability` |
|---|---|---|
| default (`AllowAll`) | **connects** | — |
| `block_network()` | refused `EPERM` | **connects** |
| `proxy_only(port)` | refused `EPERM` | **connects** |

The emitted Seatbelt profile was captured directly (by interposing
`sandbox_init`, since `generate_profile` is private) and is exactly what S1
predicted: `(deny network*)`, the two mDNSResponder carve-outs, and — when a
grant is present — one added `(allow network-outbound (path "…"))` line.

Two findings beyond what S1 claimed, both of which the implementation depends on:

- **`connect()` is not gated by the filesystem layer in either direction.** With
  the network unrestricted, the sandbox reached the daemon socket *while `stat()`
  on that same path was denied*. And with the network denied, granting the
  socket's path via `FsCapability` made `stat()` succeed and left `connect()`
  refused. The two are orthogonal layers (nono #696), so a filesystem grant is
  neither necessary nor sufficient — which is what makes the network axis the
  only lever, not merely the preferred one.
- **A grant is scoped to the path it names**, not to its parent: a socket sharing
  a directory with a granted one stays refused. That is the spec's "admits its own
  proxy socket and no other sandbox's", measured.

**Alternative considered and rejected: a scoped filesystem deny.** Denying the socket's
*path* via a filesystem-mode Seatbelt rule was considered and rejected — Seatbelt's own
`(file-write*)`/`(file-read*)` operations are not what mediates a unix-domain
`connect()` per the same source reading above; a filesystem-shaped rule would very
plausibly not fire at all, and layering one on top of a working network-shaped rule
adds surface for no verified benefit.

## S2 — The proxy socket needs no grant on macOS (CORRECTED by the spike)

> **Superseded.** The decision below was written from a source reading and is
> **wrong about macOS**, not in its mechanism but in its premise. It is kept
> rather than deleted because the reasoning is still exactly right for the
> platform it was generalising from, and because task 0.4 exists to record
> outcomes like this one.

**Corrected decision.** Compile **no** `UnixSocketCapability` for the proxy on
macOS, because macOS never dials the proxy over a unix socket in the first place.

**Why the original premise fails.** The proxy binds *two* listeners
(`proxy::spawn`): a TCP loopback port and a unix socket. Which one a sandbox uses
is decided by `up`, and the unix socket is only ever dialled by path when the
`relay` is active — `let relay = isolate_network.then(…)`. `isolate_network`
requires `fleet::netns::probe`, and network namespaces are Linux-only, so on macOS
`relay` is always `None` and `HTTPS_PROXY` points at `127.0.0.1:<port>`. The
compiled policy admits exactly that, via `proxy_only(port)`, whose emitted profile
was captured on the same run as S1's:

```
(deny network*)
(allow network-outbound (remote tcp "localhost:1"))
(allow system-socket (socket-domain AF_INET) (socket-type SOCK_STREAM))
```

So the sandbox's own egress path is already open, on the TCP axis, and adding a
unix-socket grant for `proxy.sock` would compile a rule that can never fire.

**The spec requirement it was serving still holds** — "the sandbox's own egress
path stays reachable", and scoped to its own proxy — it is simply satisfied by a
different mechanism than this decision assumed. That is why the requirement is
stated as an outcome and this document, not the spec, is where the mechanism
lives.

**What this removes:** task group 1 in its entirety. There is no macOS-only branch
to add to `to_capability_set`, and adding one would be dead code carrying a
security-shaped grant.

<details>
<summary>Original S2, superseded — kept for the reasoning, which is sound for the case it describes</summary>

**Decision.** Grant devcroft's own egress-proxy unix socket explicitly, via `nono`'s
`UnixSocketCapability`/`SocketScope` primitive, compiled alongside the existing
`network.allow`-driven policy — the macOS-shaped sibling of `filesystem-view`'s own M3
requirement.

**Rationale.** Same underlying fact as `filesystem-view`'s M3 (`add-mount-isolation`
design.md): a sandbox that reaches its allowlisted hosts through devcroft's own proxy
dials that proxy over a unix socket, and a deny-default rule that reaches AF_UNIX
indiscriminately would silently cut that path too — a sandbox that starts, reports
healthy, and has no network. Unlike Linux, there is no view to include the socket's
*path* in; the fix is a rule admitting the specific socket, which is exactly what
`UnixSocketCapability`'s `SocketScope` is for — the mDNSResponder grant cited in S1 is
the library's own working example of the identical pattern.

Control and SSH sockets need nothing: both are inherited file descriptors, dialled by
the keeper before restriction and never looked up by path again afterward, the same
reasoning `filesystem-view`'s own M3 gives for why *those* two sockets survive
masking on Linux.

</details>

## S3 — The existing test's assertion is platform-specific and stays that way

**Decision.** `tests/unix_socket_not_mediated.rs`'s current assertions (`ENOENT` — the
path does not resolve at all) are Linux-specific and are not reused for macOS. A macOS
run of the equivalent property asserts `EPERM` from Seatbelt's network deny instead.

**Rationale.** The two platforms close the same *class* of gap through mechanisms with
different observable failure shapes — a mount view makes a path not exist; a network
deny makes a syscall fail against a path that still resolves. Forcing one assertion to
cover both would either be vacuously true on the platform it wasn't written for or
require weakening what either platform actually proves. Kept as two clearly-labeled
cases in the same file (mirroring how the Linux file already documents the abstract-
socket half it does *not* cover) rather than as a shared assertion.

**MEASURED, and the split turned out to need more than a different errno.** Writing
the macOS half the obvious way — cwd elsewhere, socket ungranted, mirroring Linux —
produced tests that passed *with the deny rule deliberately removed*. `/tmp` on macOS
is a symlink to `/private/tmp`, and resolving it is a filesystem read the probe had
not granted, so `connect()` failed `EPERM` during path resolution regardless of
network policy — the right errno for the wrong reason, indistinguishable from the
property under test. Two fixes, both now in the file: hand the probe the
already-resolved `/private/tmp/…` path, and run it from the socket's own directory so
the socket is filesystem-granted and only the network rule can refuse. Verified by
removing `block_network()` and confirming all three macOS tests fail, then restoring
it and confirming all three pass.

## Risks / Trade-offs

- **[Risk] The source reading is wrong or incomplete** — Seatbelt's actual behavior on
  a real kernel could differ from what the profile-generation source suggests, the same
  way three earlier claims in this project were reasonable and measured wrong (design.md
  C2, `add-backend-capabilities`). → **Mitigation**: Open Question 1's spike is the
  entire point of task 0; nothing downstream of it treats the mechanism as confirmed.
- **[Risk] `UnixSocketCapability`'s `SocketScope` doesn't compose cleanly with the rest
  of the compiled policy** (ordering relative to the network-deny rule, interaction with
  `network.allow`'s own domain rules) — unknown until it's actually built against a real
  `CapabilitySet`. → **Mitigation**: task 1 builds it host-side and inspects the emitted
  Seatbelt profile text before task 0's spike ever needs a Mac to run anything; the
  profile text itself can be reviewed on this Linux devcontainer, only the *behavior*
  needs real hardware.
- **[Trade-off] This change, if it lands, narrows but does not eliminate the
  cross-platform gap `add-mount-isolation` left.** macOS still has no filesystem-view
  equivalent (Non-Goals), so a *filesystem*-reachable exposure narrower than "which
  socket" — e.g. a project code path that doesn't go through `connect()` at all — is
  unaffected. Worth stating precisely rather than letting the AF_UNIX fix read as
  "macOS is now at parity with Linux."

## Open Questions

1. **~~The load-bearing one: does any of this actually work on macOS?~~ ANSWERED —
   yes, on both counts.** Measured on macOS 15.7.4 (arm64): (a) `network.default =
   "deny"` denies `connect()` to an ungranted pathname unix socket (`EPERM`,
   confirmed against a live `nix-daemon` socket), and (b) a `UnixSocketCapability`
   grant admits that specific socket and no other. See S1 for the full matrix and the
   captured profile text. The question's premise — that this repository has no macOS
   host to answer it on — was the thing that was actually wrong.

   One consequence worth stating, because it is not what the change expected: **the
   capability required no policy-compilation code at all.** devcroft already compiled
   `network.default = "deny"` to a rule that covers AF_UNIX; what was missing was the
   measurement and the claim, not the mechanism. The prerequisite that *did* need
   code was unrelated and unnoticed — the crate did not compile on macOS at all
   (`src/fleet/mount.rs` is Linux-only and was ungated), so no macOS behaviour of any
   kind could have been observed before this change.
2. **Does the deny-default rule reach *abstract* macOS-side sockets the same way?**
   macOS unix sockets are conventionally pathname-based; whether an equivalent to
   Linux's abstract-socket namespace exists there at all is not established by anything
   read so far. Left open rather than guessed — out of this change's stated scope
   either way (the proposal is pathname sockets specifically, matching
   `filesystem-view`'s own split of the gap into two halves).

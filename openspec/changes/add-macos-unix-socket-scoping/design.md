# Design — macOS Unix Socket Scoping

## Context

`add-mount-isolation` closed the pathname-AF_UNIX gap on Linux with a per-sandbox mount
namespace (`fleet::mount::construct_view`). That mechanism does not exist on macOS —
Seatbelt has no user/mount namespace equivalent — and an attempt to make mount
isolation unconditional across platforms briefly broke every macOS `up` before being
corrected to degrade gracefully (warn, proceed Seatbelt-only), which is where this
change starts from: macOS today is exactly as exposed to this gap as it was before
`add-mount-isolation` existed.

**Nothing in this document has been run.** This repository has no macOS host. Every
finding below comes from reading `nono` 0.74.0's own macOS sandbox source
(`src/sandbox/macos.rs`), the same discipline this project applies to every other
unverified macOS claim (`policy::degraded`'s domain-filtering doc,
`docs/threat-model.md`'s existing macOS caveats) — an argument, not a measurement, and
treated as exactly that until Open Question 1 is answered.

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

**Alternative considered and rejected: a scoped filesystem deny.** Denying the socket's
*path* via a filesystem-mode Seatbelt rule was considered and rejected — Seatbelt's own
`(file-write*)`/`(file-read*)` operations are not what mediates a unix-domain
`connect()` per the same source reading above; a filesystem-shaped rule would very
plausibly not fire at all, and layering one on top of a working network-shaped rule
adds surface for no verified benefit.

## S2 — The proxy socket needs a scoped grant, not a mount-view exception

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

1. **The load-bearing one: does any of this actually work on macOS?** Task 0, before
   anything else in this change's task list, is a spike on real macOS hardware
   confirming two things live: (a) `network.default = "deny"` genuinely denies
   `connect()` to an ungranted pathname unix socket, and (b) a `UnixSocketCapability`
   grant for a specific socket path admits it through that same deny rule. Until both
   are confirmed, this change stays a proposal — no spec requirement here is reported
   `enforced` anywhere, per the spec's own "verified before claimed" requirement.
2. **Does the deny-default rule reach *abstract* macOS-side sockets the same way?**
   macOS unix sockets are conventionally pathname-based; whether an equivalent to
   Linux's abstract-socket namespace exists there at all is not established by anything
   read so far. Left open rather than guessed — out of this change's stated scope
   either way (the proposal is pathname sockets specifically, matching
   `filesystem-view`'s own split of the gap into two halves).

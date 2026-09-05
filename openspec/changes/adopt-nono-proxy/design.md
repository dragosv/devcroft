# Design — Adopt `nono-proxy`

## Context

`add-egress-proxy` E6 already made this decision and deferred it on cost. The
owner has accepted the cost, so this document does not re-argue the adoption;
it records what re-measuring changed, and decides the things E6 left open.

Four measurements, all taken before any code moved:

1. **75 crates, not 116** (`nono-proxy` 0.75.0, `default-features = false`,
   resolved for `x86_64-unknown-linux-gnu`).
2. **33 of those 75 serve capabilities devcroft refuses** — 20 AWS, 6
   SPIFFE/gRPC, 7 TLS/certificate — and `system-keyring` is the crate's only
   feature, so none can be compiled out.
3. **`nono` must move 0.74.0 → 0.75.0**; `nono-proxy` 0.75.0 requires it, so
   E6's "pinned to the same 0.74.0" is no longer available.
4. **That upgrade is free**: `cargo check --all-targets` on nono 0.75.0 is clean,
   zero warnings, no source change. Measured on macOS 15.7.4/arm64; unverified
   on Linux.

## Goals / Non-Goals

**Goals:**
- Credential brokering: an agent authenticates to an upstream without ever
  holding the secret.
- Keep every property `add-egress-proxy` shipped — placement, fail-closed,
  per-session token, `policy --render` visibility.

**Non-Goals:**
- TLS interception, SPIFFE, AWS routing. Compiled in, unreachable, asserted so.
- Replacing the proxy's process or lifecycle (E1/E2 stand).

## Decisions

## D1 — Replace the loop, keep the process

Unchanged from E6, restated because it is what preserves the boundary argument:
`proxy::spawn`, the pidfile, `up`/`down`/`rm` ownership,
`CompiledPolicy::network_proxy_port` and the `NetworkMode::ProxyOnly` kernel
gate stay devcroft's. `nono_proxy::start(ProxyConfig)` replaces the accept loop
inside a process devcroft still owns and still restricts around.

The fail-closed property survives because it is structural: the kernel gate
permits exactly one port whether or not anything is listening on it.

## D2 — The `nono` 0.75 bump is this change's first task, and it is already green

Measured clean rather than assumed. It is still listed first because it is the
one step that can break everything else, and because a clean `cargo check` on
one platform is not the same as a clean test run on two.

## D3 — The refused capabilities get an assertion, not a comment

E6 said TLS interception, SPIFFE and AWS routing are "present in the dependency,
unused by devcroft, and not to be enabled silently". A comment cannot enforce
that. Since none of the three can be compiled out, the enforcement has to be a
test: construct the `ProxyConfig` devcroft builds from an arbitrary manifest and
assert all three are off.

**Rationale.** The failure mode is not someone deliberately enabling TLS
interception; it is a future `ProxyConfig` default changing upstream, in a minor
release, and devcroft inheriting a capability its threat model denies having. A
test fails on that upgrade. A comment does not.

**And the upstream ask.** `use-nono-library` task 6.4 already asks this
maintainer to gate `nono`'s trust module behind a feature. This adds the second
half: gate AWS and SPIFFE too. If accepted, the cost drops from 75 crates to
about 42. Filed by the owner, as 6.4 is — an agent does not open issues on
third-party repositories unprompted.

## D4 — Where the credential comes from: the narrowest answer that works

`nono-proxy` can load from a system keystore or 1Password. devcroft takes
neither in this change. The manifest names a route and an *indirection* to the
secret; resolving it is host-side, at `up`, in the trusted phase — the same
phase and the same trust assumption as provider resolution.

**Rationale.** A keystore integration is a product decision with its own
platform matrix (Keychain, Secret Service, DBus) and its own dependency tail.
Adopting one silently inside a proxy change would be the second unannounced
adoption in one commit. The narrow answer is reversible; a keystore is not.

**Consequence to state plainly:** the credential is read on the host by
devcroft, held in the proxy process, and never handed down. That is strictly
better than an environment variable in the sandbox, and strictly weaker than a
hardware-backed store. Say the true thing.

## D5 — The agent has to be *pointed* at the route, and that is a real edge

Brokering only works if the client calls the local route. For Claude Code that
is `ANTHROPIC_BASE_URL`; for others it is some equivalent. devcroft sets it in
the resolved environment.

**The failure mode worth designing for:** a client that ignores the variable, or
a user who overrides it, reaches the real upstream directly — where the
allowlist and the kernel gate still apply, so it *fails* rather than leaking.
That is the right failure, and it must be legible: the refusal has to say the
upstream was not reachable because it is brokered, not merely that egress was
denied.

## D6 — Keep devcroft's per-session token, and set `strict_connect_auth: true`
against the crate's default

**Decision.** `require_auth: true`, `session_token: Some(devcroft's token)` —
devcroft keeps minting it in `proxy::spawn` and delivering it as
`DEVCROFT_EGRESS_TOKEN`, so the lifecycle stays devcroft's and only the checking
moves. And `strict_connect_auth: true`, which is **not** the crate's default.

**Why the default is wrong here, measured rather than argued.** The crate
defaults `strict_connect_auth` to `false`, and its own doc gives the reason:
*"Node.js undici does not echo URL-userinfo credentials as `Proxy-Authorization`
on CONNECT, and the sandbox itself is the trust boundary there."* devcroft's
proxy refuses unauthenticated CONNECT with `407`, and `authorized()`'s doc names
`curl`, `git` and `npm` as clients that work unmodified — so if that claim held,
devcroft would already be broken for every Node-based agent, which is the exact
workload `add-agent-workload` targets.

It does not hold on the current stack. Measured against a listener that prints
the CONNECT request headers, Node 22.22.3 with undici 8.10.2:

| client | `Proxy-Authorization` on CONNECT |
|---|---|
| `curl` 8.7.1 | yes |
| `undici` 8.10.2 via `ProxyAgent(url-with-userinfo)` | **yes** |

So devcroft's existing strict behaviour is correct and stays. **Taking the
crate's default would have been a silent security regression** — CONNECT
tunnelling without authentication, reopening precisely the open-relay hole
`add-egress-proxy` task group 4a closed. That is D3's hypothetical risk turning
out to be concrete on the very first config field, which is the argument for
D3's assertion rather than a comment.

**Scope of the measurement, stated so it is not over-read**: one client stack,
configured with an explicit `ProxyAgent` over a userinfo URL. A proxy configured
some other way — Node 24's env-driven `fetch`, a different agent's HTTP
library — may behave differently, and the upstream comment may simply predate
undici 8. This decision rests on devcroft's own behaviour being preserved, not
on undici being universally well-behaved.

## D7 — `nono_proxy::start` cannot serve devcroft's unix-socket path

**The blocker, found while scoping D1's swap.** devcroft's proxy is reachable
two ways, and only one of them is TCP:

1. a loopback `TcpListener`, bound in `proxy::spawn` and fd-passed to the child;
2. a **unix socket** (`StatePaths::proxy_socket`, 0600), because a
   network-isolated sandbox has a loopback-only namespace and cannot reach the
   host's `127.0.0.1` at all. Egress there is a unix-socket relay.

`nono_proxy::start(ProxyConfig)` binds its own TCP listener from
`bind_addr`/`bind_port`. It accepts neither a pre-bound fd nor a unix socket.
So D1's "replace the loop" is **not a drop-in**, and task 1.1 as originally
written is wrong.

**Decision: keep devcroft's unix-socket acceptor in the same process, splicing
to a loopback `nono-proxy` it starts itself.** The relay stays devcroft's — it
is the netns design, not proxy behaviour — and everything the relay carries
still passes through the crate's auth, filtering, brokering and audit.

**Rejected: keep devcroft's loop for the unix path and use the crate for TCP.**
Two implementations of the same policy decisions, diverging on the first
upstream change, and the unix path is the one fleet actually uses.

**Cost:** one extra in-process hop for the namespaced path. Worth measuring
rather than assuming negligible, since that path is the fleet path.

**Also worth asking upstream** — a `start_on_listener` taking a pre-bound
`TcpListener` or `UnixListener` would remove the hop and the relay. Not folded
into `docs/nono-feature-gating-issue.md`: that one asks for feature gates and
is nearly free to grant, and bundling an API request would weaken it.

## Risks / Trade-offs

- **[Risk] Risk concentration.** After this, devcroft's boundary, egress filter
  and credential handling all come from one upstream with one author.
  `use-nono-library` accepted that for the boundary; this extends it. →
  **Mitigation**: none that removes it. Stated in the proposal and in the README's
  dependency posture rather than discovered by a user.
- **[Risk] A minor upstream release changes a `ProxyConfig` default** and
  devcroft silently gains a refused capability. → **Mitigation**: D3's assertion.
- **[Trade-off] 33 crates for capabilities that are switched off.** Accepted,
  with the upstream ask filed. The alternative — writing credential injection,
  phantom-token shaping and audit integrity in devcroft — is a large amount of
  security-sensitive code to own, which is E6's original argument and still holds.
- **[Risk] The attribution file grows by 75 entries** and must be regenerated
  **on Linux**; doing it on macOS silently drops the Linux-only tail from a
  file that exists for Apache-2.0 §4(a) compliance.

## Open Questions

1. ~~Does the per-session token survive the swap?~~ **Resolved in D6**: devcroft
   keeps minting it, the crate checks it, both return `407`, and
   `strict_connect_auth` is forced on against the crate's default.
2. **What happens to `why --host`?** It answers from devcroft's compiled policy
   today. Once the L7 endpoint policy exists, a host can be allowed while a
   *method and path* on it are not, and `why` would be answering a coarser
   question than the proxy decides. Not resolved here.

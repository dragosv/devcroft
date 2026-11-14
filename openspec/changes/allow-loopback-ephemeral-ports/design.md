# Design — Loopback ports chosen at runtime

## Context

See proposal.md, "Why", for the motivation (`agy` binds `127.0.0.1:0`).
The specs are in `specs/network/spec.md`.

How a `network.default = "deny"` sandbox is confined today, on Linux:

- **A network namespace of its own** (`CompiledPolicy::wants_network_isolation`,
  for every deny sandbox). It has no route out: loopback only. Egress reaches
  the host proxy through the keeper's relay. The relay listens on the proxy's
  port number inside the namespace and forwards over a pathname unix socket,
  which crosses namespaces (`add-egress-proxy` E7).
- **Landlock `NetPort` inside it** (`CapabilitySet::apply_network_and_signal`).
  It allows `proxy_only(port)` or `block_network()`, plus
  `allow_localhost_port(p)` for each `network.ports` entry and each declared
  service port. Landlock rules are one port each. There is no range, and no
  "any".
- **The namespace is best-effort for `up`.** If `fleet::netns::probe` fails,
  or the proxy's port collides with a declared one, `up` warns once
  (`warn_network_isolation_degraded`) and runs on the host's shared loopback.

`add-linux-agent-fleet` D9 (amended 2026-09-28) established that in this
topology the route-less namespace is the egress boundary, for every protocol
and address. `NetPort` is a port filter inside it. UDP was never filtered by
Landlock and is closed only by the namespace (`wants_network_isolation`'s
comment). This change relies on that finding. It does not establish it.

macOS has no namespace. Its sandboxes share the host's loopback, and Seatbelt
is the only filter.

## Goals / Non-Goals

**Goals:**
- A deny sandbox that opts in can bind port 0 and connect to any loopback
  port inside its own namespace.
- Nothing outside that namespace becomes reachable. This is shown by a test,
  not argued.
- Every place the namespace is absent refuses the key.

**Non-Goals:**
- **Port ranges, or any form of port selection on a shared loopback.** That
  is `add-port-allocation`'s territory and a different question.
- **macOS support.** It is refused, and listed in `docs/known-gaps.md`.
- **Fleet agents.** Fleet builds its own per-agent namespace and has its own
  network spec (`agent-networking`). Whether the key applies there is
  decided when fleet consumes it, not here.
- **Changing what `network.ports` means.** It still grants named ports, and
  `network.services` still decides host exposure.

## Decisions

### L1 — On Linux, `"any"` drops Landlock's TCP port filter inside the namespace

**Decision.** With `network.loopback = "any"`, the compiled capability set
does not restrict TCP bind or connect (`NetworkMode::AllowAll` at the
Landlock layer). The boundary is the route-less namespace, and `up` verifies
that the namespace is in place before the keeper restricts itself (L3).

**Why not a port-0 rule.** Landlock may allow an ephemeral bind given a rule
for port 0; group 0 measures this on this host's ABI. Even if it does, it
covers only half of what `agy` does. The *connect* to the port the kernel
chose would still need a rule for that exact port, which cannot be written
before the port exists. Bind-only handling (leaving connect unhandled) is
the same result as L1, reached by a less obvious route.

**Why this is not a widening.** Inside a route-less namespace, the TCP
destinations are:

- the sandbox's own listeners;
- the relay, on the proxy's port, which is already granted;
- anything else devcroft puts there. Group 0 enumerates these, rather than
  this document asserting there are none.

Everything outside is unreachable at the routing layer, whatever Landlock
says. `NetPort` was already redundant for egress there. What it still did
was stop the sandbox's *own* processes from binding and connecting on ports
nobody declared, and that is exactly the restriction being lifted, by
request.

**Rejected: seccomp, or a userspace port broker.** Both would emulate a
boundary the namespace already provides. CLAUDE.md records that adding a
helper with a route reinstates D9's gate. A helper without one adds nothing.

### L2 — A string key with two values, opt-in

`network.loopback = "declared" | "any"`, default `"declared"`.

- **Opt-in** because the default must keep every compiled profile
  byte-identical. That is the policy-determinism invariant, and it keeps
  this change inert for existing projects.
- **A string, not a boolean** like `any_loopback_port = true`. A third
  value, such as a declared range on a shared loopback, would otherwise need
  a second key that interacts with the first.
- **Under `[network]`**, beside `ports`, which it generalizes. The key
  allowlist in `config::validate` gains `loopback`.

### L3 — Refuse in three places, each at the layer that knows

| condition | layer | when |
|---|---|---|
| `default = "allow"` | `config` | parse/validate; it is static |
| macOS | `backend` | `up`, before state is written; known at compile time |
| no namespace on this Linux host, or the proxy-port collision that skips isolation | `backend` | `up`, where `isolate_network` is decided |

The third row is a behaviour change only for manifests that set `"any"`.
Today an unavailable namespace *degrades* with a warning. With `"any"` it
*refuses*, because degrading would turn "any port on my loopback" into "any
port on the host's loopback". That is a wider grant than the one written,
which "Provider resolution must not widen the policy" forbids in spirit and
the spec forbids in words. The proxy-port collision is handled the same
way: refuse, naming `network.ports`, rather than silently skipping the
namespace. A re-run draws a new proxy port, and the message says so.

**Route-less, verified rather than assumed.** Where the namespace is
created, `up` confirms that it has no default route before the keeper
applies its profile. That is a single routing-table read. If a future
topology adds a route (slirp4netns, pasta), `"any"` fails closed instead of
quietly becoming a hole. It is a single check that makes D9's precondition
explicit at the one place this change depends on it.

### L4 — Rendering

`policy --render` gains one network line:
`loopback: any port, own namespace only  [manifest:network.loopback]`. It
replaces nothing, so `network.ports` lines and the proxy-port line still
render. A future reader can then see that the port list is moot under
`"any"`, and why. (`why` has no port question at all, only paths and hosts,
so there is nothing for it to answer here. Found while implementing; task
2.3 was struck rather than inventing a command surface.)

### L5 — The agents follow-up lives here, not in a separate change

The sample swap (`devenv-gemini-sample` → `devenv-antigravity-sample`) and
the `docs/agents.md` row are what prove the key does its job against the
program that motivated it. A separate change would have to restate this
one's context to justify itself. The settings the sample needs are already
measured (proposal.md): `modelProvider = "gemini"`, written by `enterShell`
into the sandbox's home; `GEMINI_API_KEY` through `forward`; and
`generativelanguage.googleapis.com` allowed.

## Measured (group 0, 2026-11-14, Landlock ABI 10, aarch64 Linux)

**0.1 — the failure.** Running `agy -p` in `devenv-antigravity-sample`
(agy 1.4.1 from `devenv-nixpkgs/rolling`) exits before any request. The
sandbox home's `cli.log` says `CLI failed to start - listen tcp 127.0.0.1:0:
bind: permission denied`, from `language_server.setUpServerPortListeners`.
The proxy log is empty: nothing left the sandbox.

**0.2 — what is inside a deny sandbox's namespace.** The sandbox had an
allowlist (`example.com`), so the relay exists, and a supervised service on
8730. Read from inside a session:

| | found | verdict |
|---|---|---|
| TCP listeners | `127.0.0.1:45531`, the relay on the proxy's port (`policy --render`: `network.proxy: 127.0.0.1:45531`) | harmless: already granted under `"declared"`, and it leads only to the proxy, which checks the token and the allowlist |
| | `127.0.0.1:8730`, the sandbox's own service | the sandbox's own |
| interfaces | `lo` only | |
| IPv4 routes (`/proc/net/route`) | none | route-less |
| IPv6 routes | only the kernel's default `lo` entries | route-less |

Nothing devcroft places in the namespace becomes newly reachable under
`"any"` except the sandbox's own listeners.

**0.3 — L1's mechanism, without devcroft.** In an unprivileged user+net
namespace with `lo` brought up, no Landlock, and an empty routing table:

- bind `127.0.0.1:0` worked, and connecting to the port it returned worked;
- a connect to `1.1.1.1:80` failed with `ENETUNREACH`;
- a connect to `127.0.0.1:18999`, where a host process was listening on the
  host's loopback, failed with `ECONNREFUSED`, because the namespace's
  loopback is a different interface.

The control on the host reached all three. The first attempt forgot to
bring `lo` up and read as `ENETUNREACH` for everything, which says nothing.
A probe with `lo` down is not a measurement.

**0.4 — the rejected alternative.** On Landlock ABI 10, with net bind and
connect handled:

- with no rule, bind on port 0 was refused (`EACCES`);
- with a bind rule for port 0, the bind succeeded, and the *connect* to the
  returned port was refused (`EACCES`).

A port-0 rule is real, and it is half of what `agy` needs, as L1 says.

**0.5 — the nono call.** nono 0.77 `sandbox/linux.rs`
`prepare_with_abi_inner` handles Landlock net only when the mode is not
`AllowAll`, or when explicit `tcp_connect_ports`/`tcp_bind_ports` exist.
`localhost_ports` is ignored under `AllowAll`. So `"any"` compiles to
`set_network_mode(NetworkMode::AllowAll)`, with no `allow_localhost_port`
calls and no explicit TCP port lists. The handled set is then empty, and no
seccomp fallback is installed, since one is chosen only when net handling
is needed. nono also *refuses* `open_port 0` on Linux ("localhost TCP
wildcard is macOS-only"), where it means the Seatbelt `localhost:*`
wildcard. That confirms the macOS spelling exists and that it means the
host's loopback, which L3 refuses.

The filesystem profile does not depend on the network mode. The only other
consumer of `network_proxy_port` is the proxy environment the keeper hands
to sessions, which is separate from the capability set, so `"any"` leaves
egress configuration intact. Task 2.1 must keep it that way.

## Risks / Trade-offs

- **[Risk] Something other than the sandbox listens inside the namespace**,
  and `"any"` makes it reachable when it was not before. → Group 0
  enumerates the namespace's listeners with the keeper and relay up. Each
  one found is named in this design, as either harmless (the relay) or a
  reason to stop.
- **[Risk] The namespace gains a route in some future topology**, making L1
  a real egress hole. → L3's route check fails `up`, and the spec's "never
  reaches beyond the sandbox" scenarios are tests that would fail too.
- **[Trade-off] On Linux, `"any"` removes the only TCP-port-level evidence
  in the profile.** A reader of `profile.json` sees no net rules and must
  know the namespace is the boundary. → L4's rendered line says so in
  words, and `policy --render` is the documented inspection surface, not
  `profile.json`.
- **[Trade-off] `agy` will not run on macOS.** → It is refused by name and
  listed in `docs/known-gaps.md` and in `docs/agents.md`'s row. A Seatbelt
  route (`localhost:*`) exists but reaches every host service, so taking it
  would be the silent widening this change refuses.

## Migration Plan

None. The key is new and opt-in. Rollback is removing the key from a
manifest.

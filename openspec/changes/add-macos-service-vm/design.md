# Design — add-macos-service-vm

Every measurement quoted here was taken on macOS 15.7.4 / arm64 during the
investigation that produced this change, not read from documentation.

## Decision 1: the VM is shared, the services are not

Rejected: **a VM per worktree** — more lifecycle, slower start, and no
reuse of the Linux store. Rejected: **shared services across worktrees** —
that reintroduces exactly the cross-branch interference the change exists
to remove; two branches sharing one Postgres is worse than two branches
fighting over a port, because the failure is silent.

So one guest, and inside it a network namespace, a supervisor and a data
volume per sandbox. This is the same mechanism `fleet::netns` already
implements on Linux, which is why it is reused rather than reinvented.

**A single namespace in the guest is not enough and it is the obvious
mistake**: it moves the collision from macOS into Linux and looks like
progress until the second `up`.

## Decision 2: the worktree source is *not* mounted into the guest

The reviewed sketch mounted each worktree into the VM. This change does
not, and dropping it removes a cost and a risk at once.

**A Postgres needs a data directory, not your source tree.** Nothing the
services do requires the project's files, so mounting them would pay the
~2× virtiofs penalty measured above for no benefit — and would put both
worktrees' sources inside one shared guest, where a service in sandbox A
could reach sandbox B's code. That cross-worktree path does not exist
today and this change must not create it.

If a future service genuinely needs project files, that is a separate,
argued addition with its own mount-namespace story — not a default.

## Decision 3: the guest is Nix and SSH, nothing above

The guest ships a Linux Nix store and an SSH account. It does **not**
ship flox, devenv or devbox.

Each of those has its own environment model *and its own service
mechanism*. A guest that supported all three would stop being a VM and
become a platform, and every tool upgrade would become devcroft's problem.
Instead the worktree brings its own tool into the shared store, where the
second installation of an identical closure is free.

Consequence accepted: a slower first activation per worktree. Gained: the
guest has no opinions, so there is nothing in it to break when a project
changes tools.

## Decision 4: two resolutions, named as such

This is the invariant this change puts under strain, so it is called out
rather than discovered later. CLAUDE.md: **"Environment resolves once, at
`up`."** Today one provider resolution yields env, grants and services
together.

Here the toolchain resolves for **Darwin** (host, unchanged) and the
services resolve for **Linux** (guest). The lockfile can be shared; the
closures cannot — a Linux binary does not run on macOS.

The rule is preserved by *not* pretending this is one resolution: the
Linux service environment is a **second, separately recorded** resolution
in `Meta`, with its own fingerprint and its own staleness answer. What must
never happen is a single `Resolution` that silently means different things
on different sides.

## Decision 5: the host-side endpoint is per sandbox, and unix sockets first

The application on macOS reaches its service through an endpoint devcroft
owns:

```
native macOS process
  -> per-sandbox endpoint (unix socket, or an allocated 127.0.0.1 port)
  -> devcroft relay
  -> the sandbox's network namespace in the guest
  -> the service on its declared port, e.g. 5432
```

A unix socket is preferred where the client supports one (PostgreSQL does)
because it is identified by the sandbox rather than by a globally scarce
port, and its lifetime is the keeper's. A loopback port is the fallback for
protocols that need TCP, allocated per sandbox and shown in `status`.

**The manifest stays true where it is read**: inside the guest the service
is on the port the project declared. Only the host-side mapping is
allocated, and it is displayed rather than implied.

## Decision 6: the existing `Supervisor` seam is on a different axis

`decouple-service-supervisor` shipped `trait Supervisor` — `binary`,
`render_config`, `spawn_args`, `query` — which abstracts *which supervisor*
runs, assuming it runs in "the resolved environment".

This change is about *where* services run. The two are orthogonal, and
conflating them would make `Supervisor::binary()`'s contract ambiguous:
process-compose must be present in the **Linux** environment, not the
Darwin one. Whatever seam this change adds must say which environment it
means, and must not quietly redefine the existing trait's.

## Decision 7: Lima is a backend, not a concept

Lima is the first implementation: mature on macOS, uses
Virtualization.framework, manages guest lifecycle. It must sit behind a
devcroft-owned seam so the model does not leak Lima's vocabulary.

The standing rule was consulted rather than assumed: *"the keeper SHALL NOT
be executed as a child of a separate sandboxing binary"* — the rule that
already refused firecracker and bubblewrap. It passes here, because Lima
runs the **services**, not the keeper, and the keeper stays a direct child
of `up`. That argument belongs in the record, since this is the third time
the rule has been applied.

Accepted cost, stated plainly: devcroft has **no runtime dependency on
macOS today**. Lima would be the first.

## Decision 8: what a dead VM must look like

If the guest is gone, `status` reports services unavailable. It must not
report the sandbox healthy — the same rule that governs every other
degraded capability. `down` stops services and endpoints but keeps data;
`rm` deletes the data volume. Data outliving `down` is the existing
contract and this change does not get to invent a different one.

## Rejected outright

- **Loopback aliases per sandbox** — needs root (`ifconfig lo0 alias`
  returns `permission denied`), and only helps a service binding that
  specific address: `bind("0.0.0.0")` still collides, measured.
- **`SO_REUSEPORT`** — both sockets bind and the kernel load-balances
  between them. It fails by working, which is worse than failing.
- **Putting the code in the VM too.** It is the only arrangement where
  `localhost:5432` keeps working unchanged, and it costs the native macOS
  toolchain — including the `swift` provider, which exists precisely
  because it is macOS-native.

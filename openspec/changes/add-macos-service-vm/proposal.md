# Change: add-macos-service-vm

Status: proposed (post-MVP). Depends on: `add-flox-services` (complete),
`decouple-service-supervisor` (complete), `fleet::netns` (implemented).
Supersedes the surviving scope of `add-port-allocation`.

## Why

**On macOS, two worktrees of one repository cannot both run the project's
Postgres.** Measured on macOS 15.7.4, two real git worktrees brought up as
separate sandboxes, both declaring `ports = [17777]`:

```
wta: hold.py    -> HELD
wtb: probe.py   -> BIND-FAILED errno=48 Address already in use
host: probe.py  -> BIND-FAILED errno=48 Address already in use
```

The host's own `lsof` sees the listener the sandbox created: nothing
separates the two sandboxes' loopback, so the second is refused exactly as
any third process would be. On Linux this does not happen — `fleet::netns`
gives each sandbox its own port table, and its doc comment states the
property this change is trying to preserve: *"Nothing negotiates,
allocates, or rewrites anything."*

macOS has no network namespace and nothing substitutes for one. Four
alternatives were measured before proposing a VM, and all four fail:

| mechanism | measured result |
|---|---|
| extra subnets / loopback aliases | separate *specific* addresses do work — but `bind("0.0.0.0")` collides with `bind("127.0.0.1")` on the same port, and the wildcard is the common default. Also needs root: `ifconfig lo0 alias` returns `permission denied`. |
| `SO_REUSEPORT` | both sockets bind **and the kernel load-balances connections between them** — each worktree would serve a random half of the other's traffic. It fails by working. |
| `pf` redirect per sandbox | `pf` filters by uid/gid, not per process. Two worktrees run as the same user. |
| NetworkExtension / Network.framework | all operate on *flows* (`connect`, packets). The collision is at `bind`. `NETransparentProxyProvider` is per-*application*, and two worktrees run the same binary. |

The reason is structural and worth stating once: **a subnet gives more
addresses inside one binding table; the collision is a property of the
table.** Separating two workloads needs a second table — a network
namespace, or a VM. `apple/container` reaches the same conclusion from the
other direction: a lightweight VM per container, each with its own IP,
explicitly so that reaching a service "removes the need to map individual
ports". If a lighter mechanism existed, that is the team that would have
used it.

## What this change does, and the scope decision behind it

The scope is deliberately narrower than "run devcroft in a VM".

**Services move into a shared Linux VM. Code does not.** The editor, the
compiler, the agent and every build stay native on macOS under Seatbelt,
because that is the loop a developer runs a thousand times a day and it is
the one thing a VM would tax. Measured, writing 4000 small files:

| | time |
|---|---|
| native macOS (APFS) | 0.26s |
| virtiofs (host↔guest share) | 0.52s — ~2× slower |
| the guest's own filesystem | 0.14s — ~1.9× *faster* than native |

That table is the whole architecture in miniature: the VM is not slow, the
*share* is. So nothing that matters crosses it.

**The VM is shared; the services are not.** Each sandbox gets its own
network namespace, its own supervisor and its own data volume inside the
guest. A single namespace would move the collision from macOS to Linux
rather than solve it.

**Two things are given up explicitly, and the owner has accepted both:**

- The host-side application is told its database endpoint (an injected
  `DATABASE_URL` / port) rather than finding Postgres on `localhost:5432`.
  **No design can avoid this on macOS** — the client and the service would
  have to share a binding table, which means the client in Linux too, which
  means losing the native macOS toolchain. Accepted: running the app on a
  non-default port is not a problem.
- The service's own declared port is what stays untouched. That is the
  entire benefit: a Postgres declared on 5432 keeps 5432 inside its
  namespace, and devcroft never rewrites the project's service config.

## What this is *not*

- **Not a security boundary for the agent.** Host-side code is confined by
  Seatbelt exactly as today; the VM holds services, not the workload. The
  README must not imply otherwise.
- **Not a replacement for `forward = ["PORT"]`.** That already ships and
  already solves the dev-server case for services that read an env var, at
  zero cost and with no VM. This change exists for the case it cannot
  reach: a service whose port comes from its own configuration file.
- **Not for the `swift` provider.** A macOS desktop application declares no
  services — `swift` reports `ServiceSupport::Unsupported`. This change
  serves web and backend projects developed on a Mac.

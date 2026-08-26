# Add Linux Agent Fleet Support

**Depends on:** `add-egress-proxy` for the `agent-networking` capability, and
`add-backend-capabilities` — fleet declares the capabilities it requires
(`fleet`, `service_ports`, `resource_limits`, `process_isolation`) rather than
assuming a backend. The other capabilities here are independent of the proxy.

## Why

Today a devcroft environment is built for a single developer running a single
agent. Running N agents concurrently on one Linux host breaks in ways that
per-agent path isolation cannot fix:

- One agent running `make -j$(nproc)` or a forking test suite starves every
  other agent and the host. Landlock and seccomp have nothing to say about CPU,
  memory, PIDs or IO.
- Services are a core product thesis. N environments each wanting to bind
  `:5432` collide. Dynamic port rewriting destroys reproducibility.
- Agents share a PID namespace, so agent A can enumerate and signal agent B's
  processes. Landlock only gained signal scoping in ABI v6 (kernel 6.12); the
  sandbox layer targets ABI v5.
- Git worktrees share an object store and refs. A `gc.auto` run triggered inside
  one worktree can prune objects another agent is mid-write on, producing
  failures the agent will misdiagnose and "fix".
- A single network proxy cannot attribute a request to an agent, so the
  effective allowlist becomes the union of every agent's policy.

The sandbox library (`nono`, consumed as a crate) already covers the intra-agent
layer well: Landlock path rules, domain-allowlisted egress, phantom tokens
backed by the OS keychain, and per-session snapshot with accept/rollback. None
of that is replaced. This change adds the **inter-agent** layer around it.

## What Changes

- **NEW** `agent-supervisor`: a long-lived process that owns agent lifecycle,
  spawns agents via a re-executed single-threaded init helper, and exposes
  list/start/stop/status.
- **NEW** `resource-control`: one delegated cgroup v2 scope per agent, providing
  limits, atomic teardown via `cgroup.kill`, and metrics.
- **NEW** `sandbox-runtime`: namespace construction (net, pid, ipc, uts, mount)
  performed directly via `clone`/`unshare`, with the Landlock ruleset built in
  the parent and applied in the child after mounts. **Bubblewrap is explicitly
  not used.**
- **NEW** `agent-networking`: unprivileged connectivity inside each network
  namespace, with the egress proxy hosted *outside* the sandbox and reached via
  a forwarded port.
- **NEW** `service-ports`: a port model where a service declares its in-namespace
  port and optionally a host-side mapping.
- **NEW** `workspace-isolation`: per-agent git clones backed by a shared bare
  mirror, and safe concurrent use of the Nix store and daemon.

## Impact

- Affected specs: `agent-supervisor`, `resource-control`, `sandbox-runtime`,
  `agent-networking`, `service-ports`, `workspace-isolation`
- Fleet is **Linux-only**. macOS remains scoped to single-developer local work as
  a devcontainer replacement; macOS users who want fleet run it inside a Linux
  VM. The port model is shared between the two paths so config does not fork.
- Fleet serves the trusted-code use case: many instances of code the user or
  their organisation controls, where the threat is accident and interference.
  It does not extend devcroft to running code written to escape. See
  [docs/threat-model.md](../../../docs/threat-model.md).
- The existing two-tier isolation model (process / hardened) should be
  re-examined: with fleet added there are now two axes, and the tiers may
  collapse into single-agent / fleet.

## Non-Goals

- Remote or multi-host execution. Fleet is local to one Linux host.
- Replacing any capability the sandbox crate already provides.
- Syscall filtering. Evaluated and deferred to `add-syscall-filtering`
  (see `design.md`, D9).

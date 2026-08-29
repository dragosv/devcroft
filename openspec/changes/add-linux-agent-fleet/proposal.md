# Add Linux Agent Fleet Support

**Depends on:**

- `add-egress-proxy` — for `agent-networking`, and for the proxy-only seccomp
  filter D9 now makes mandatory rather than optional. **Shipped**, including
  that filter's policy model; the listener-FD handoff remains fleet's own
  phase-0 gate.
- `remove-gvisor-backend` — **shipped.** Fleet is built against the one
  remaining sandbox boundary; nothing here branches on an isolation tier.
- `add-backend-capabilities` — fleet declares the capabilities it requires
  (`fleet`, `service_ports`, `resource_limits`, `process_isolation`) rather
  than assuming a backend. **This change does not exist yet**; it is
  `remove-gvisor-backend`'s one open task, deliberately left because a
  capability matrix for a single backend is a design question rather than
  cleanup. Fleet is where it stops being optional.
- `sandbox-provisioning` — **newly declared, and load-bearing.** Fleet resolves
  a provider environment per agent. If that resolution runs unconfined on the
  host, repository-controlled code executes *before* any fleet boundary exists,
  which defeats the entire point of the per-agent namespaces below. That change
  also settles who may hold `nix-daemon` authority (its P2a/P2b), which fleet
  needs because N agents sharing one host-global store is precisely the case
  where handing a project hook that authority is worst.
- `add-agent-workload` — for the agent's own tooling and credentials. Fleet
  runs coding agents; it does not define what a coding agent *is*, how its
  runtime is declared, or how its credentials reach it. See "Composing with
  the agent workload" below.

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

## Positioning

Fleet is a **local runtime for trusted code**, not a cloud sandbox platform. It
fills the gap between plain git worktrees (which share ports, processes,
`target/`, `TMPDIR` and provider state) and remote VM or container fleets
(Docker Sandboxes, Daytona, E2B — which solve a different problem, for
untrusted code, at the cost of not being your machine).

What the combination buys that neither end of that range does:

- **Identical service ports with no command rewriting.** Every agent's Postgres
  is on 5432. Nothing is allocated, injected, or substituted into a command
  string, because each agent has its own port table.
- **Per-agent resource, workspace, process and egress control.** One runaway
  build does not starve the fleet; one agent cannot enumerate or signal
  another's processes; each agent's allowlist is its own.
- **A declarative, inspectable environment and policy.** The environment comes
  from a lockfile, and `policy --render` shows every rule with its origin —
  including the rules devcroft chose rather than the user.
- **SSH-native access with no per-IDE integration.** `ssh a17.myrepo.devcroft`
  works with any SSH-capable editor.
- **Both closure and qualified artifact providers**, rather than an image
  someone has to maintain.

**Per-agent services are an MVP requirement, not a nice-to-have.** Without them
fleet is a worktree orchestrator with better process isolation — useful, but not
the thing the proposal argues for. "Each agent gets its own Postgres" is the
claim; a fleet where agents share the host's database has not made it.

## Composing with the agent workload

Fleet runs coding agents. It deliberately does not define what one *is* — that
is `add-agent-workload`'s subject, and the two compose rather than overlap:

```
fleet          →  the agent gets a private /workspace, its own netns, its own
                  cgroup leaf, its own service stack, its own SSH endpoint
agent-workload →  what actually runs in there: the agent's tooling environment
                  (claude, codex, git), declared and lock-backed like any other
                  environment, plus how its credentials reach it
provisioning   →  how both of those environments get materialized without
                  running project code on the host first
```

The intended end state, for concreteness:

```
devcroft fleet up --agents 3
ssh a17.myrepo.devcroft 'cd /workspace && claude --task "Implement task 2.1"'
ssh a18.myrepo.devcroft 'cd /workspace && codex --task "Review a17's changes"'
ssh a19.myrepo.devcroft 'cd /workspace && cargo test'
```

Three constraints this composition imposes, worth stating before either change
is built, because getting them wrong is expensive later:

1. **The agent's tooling must not be a host binary bind-mounted in.** That would
   reintroduce `host` passthrough — the thing `docs/decisions.md` rejects by
   design — and make an agent's behaviour depend on the operator's machine. It
   is a declared, lock-backed environment or it is not reproducible.
2. **Composition order must be explicit.** Base runtime → project environment →
   agent tooling → manifest `env.vars` as the final override. Otherwise a
   `node`, `python`, `git` or `cargo` from the tooling layer silently shadows
   the project's, and the failure looks like the project's fault.
3. **Credentials need a real mechanism, not a name.** `add-agent-workload`
   currently refers to "the backend's credential mechanism"; the process
   backend has no credential broker, so today that would be ordinary
   environment injection inherited by every keeper descendant. Either a
   transport is designed or the exposure is documented — calling inherited env
   vars a credential mechanism would be the same species of overclaim this
   project treats as a defect elsewhere.

## Relationship to the nono CLI

Fleet consumes the `nono` **library** as an enforcement primitive. It does not
compose `nono wrap` subprocesses, and could not.

The ordering fleet needs — prepare the Landlock ruleset and seccomp filter in
the supervisor, construct namespaces and mounts in the init helper, *then* apply
the restriction, then start the keeper — has no expression as a CLI wrapper. A
wrapper applies its policy and execs; there is no seam to enter namespaces, run
the identity handshake, mount a workspace and become PID 1 in between. This is
the same conclusion `use-nono-library` reached for the single-sandbox case, for
the same structural reason, one layer further out.

Responsibilities, explicitly:

| component | owns |
| --- | --- |
| `nono` library | the enforcement primitives (Landlock ruleset, seccomp filters) |
| devcroft policy compiler | the baseline and the manifest → policy projection |
| fleet supervisor | identity, cgroups, namespaces, clones, ports, proxy attribution |
| per-agent keeper | sessions, services, SSH/SFTP inside one agent |

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
  port and optionally a host-side mapping. **Specified**
  (`specs/service-ports/spec.md`) — the first of this change's capabilities
  other than `agent-supervisor` to have normative requirements rather than
  only tasks. It divides explicitly with `add-port-allocation` rather than
  competing: that change allocates because today's sandboxes share the host
  loopback, and its own spec exempts a sandbox with its own network
  namespace — which is exactly the case fleet creates. Neither allocates an
  in-namespace port; only the optional host mapping is allocated, and only
  by fleet.
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
- **Not a replacement for microVM or container fleets running hostile code.**
  The boundary is the process sandbox plus namespaces plus cgroups; the full
  host kernel surface stays reachable, so a kernel bug is an escape. Fleet
  makes N trusted agents practical on one machine — it does not move devcroft
  into the use case `docs/threat-model.md` marks as not backed.
- **Resolved by `remove-gvisor-backend`, which landed first.** This entry
  asked for the two-tier model (process / hardened) to be re-examined, since
  fleet added a second axis. There is now one tier, so the isolation axis has
  collapsed on its own and the only remaining axis is single-environment
  versus fleet — which is what this change is about.

## Non-Goals

- Remote or multi-host execution. Fleet is local to one Linux host.
- Replacing any capability the sandbox crate already provides.
- **General** syscall-surface hardening. Deferred to `add-syscall-filtering`
  (`design.md`, D9). Note the narrowing: the *proxy-only* seccomp filter is
  **not** deferred — D9 reversed on that, because once a userspace network
  helper provides a general stack inside the agent, proxy environment variables
  are cooperative and the filter is the only thing that makes egress
  enforceable at all.
- Package-manager daemon access, or a writable host-global store, for agents.
  Agents receive resolved runtime paths read-only
  (`sandbox-provisioning` P2a/P2b). A workflow needing either is refused rather
  than accommodated.
- Snapshots. Per-agent clones give isolation without them; snapshotting is an
  optimisation deferred out of the MVP.
- Remote execution and multi-host scheduling — a restatement of the first
  bullet, made explicit because "fleet" invites the comparison to cloud
  orchestrators. See Positioning for what this is instead.

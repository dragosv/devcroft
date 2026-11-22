## Context

Source: the adversarial review of 2026-11-19 (family 4 in its comparison: two
sibling trees, an RPC `ProjectExecutor` and a private package). This change
takes that recommendation, on top of `add-agent-packages`. The review's
proposed acceptance matrix (R1–R7, W1–W4, L1, P1, S1–S2, U1, G1, N1, A1) is
the basis of group 0 and of the specs. `add-agent-packages` covers P1
(private package), G1 (provider untouched), A1 (ACP endpoint) and U1 (version
pinning and rollback, its design A1). S1 (state separation) is not covered
there: in mode 1 the agent's state is the shared sandbox `HOME`. This change
covers it (E6).

Facts verified on 2026-11-22 against the code:
- `SpawnRequest` (`src/keeper/protocol.rs`) carries `cmd`, `args`, `cwd`,
  `env` and `pty`, with no policy selector. The request's `env` overlays the
  keeper's.
- `up` starts one `__egress_proxy` with one allowlist and one token. Without
  a network namespace of its own, the proxy URL in every session's
  environment carries the token. Under Linux isolation it does not, and the
  unix-socket bridge presents the token instead. The broker's endpoint and
  placeholder are in every session's environment either way.
- The sandbox's `HOME` and its directory are inside the project,
  `<project>/.devcroft/<name>/`, and `own_sandbox_dir` infers "inside a
  sandbox" from `HOME`. The in-sandbox `agent` verbs and the denial hook
  depend on it.
- macOS has no `PR_SET_PDEATHSIG`; fleet's lifeline is Linux-only.

Four ACP exploration notes the review cites (`acp-agent-vm-separation-2026-11-16.md`
and siblings) are not present in any checkout. The "Form A / Form B" terms
are therefore not used here. "Mode 1" is `add-agent-packages` (one tree), and
"mode 2" is this change.

## Goals / Non-Goals

**Goals:**
- Separate policies, egress proxies and credential routes for agent and
  project, enforced by the OS on each tree.
- Every project operation a qualified harness emits through its exec tool
  runs in the project's snapshot, including direct `argv`, shell, and their
  descendants; PTY where the harness's lever carries one.
- The agent's state outside the project's reach.

**Non-Goals:**
- Intercepting every `execve`. A child the agent starts locally, bypassing its
  exec tool, stays in the agent tree and outside the guarantee (review R7).
- A second VM, or containment of hostile code.
- Filesystem RPC. The workspace is shared, so reads and writes stay local.
- Parsing ACP. If routing needs it (E4), the harness is not qualified.

## Decisions

### E1. The parent is host-side and unrestricted; the trees are restricted

`up` gains a parent process, for this mode only, that creates every listener,
socket and fd of both trees, then forks two children. Each child applies its
own profile to itself, which keeps the keeper invariant that a process
restricts itself. The parent holds no restriction and serves no session. It
supervises, restarts and tears down. On Linux it mirrors fleet's
`Supervisor`, and the decision is whether to reuse it (open question). Mode 1
sandboxes keep today's single self-restricting keeper.

Teardown must not depend on the parent surviving. Every pid of both trees and
both proxies is recorded in the state directory as it starts. `down` and the
next `up`'s orphan recovery stop each recorded process group, so a
`SIGKILL`ed parent on macOS leaves nothing running after either. On Linux the
children also hold a parent-death signal. Group 0 measures both.

### E2. The executor is bound to one pair, by a socket only the agent sees

The parent binds a pathname unix socket for the executor before restriction.
On Linux it is mounted only into the agent tree's view. On macOS only the
agent profile grants connecting to it. Any process of the agent tree,
including an MCP server the harness starts several generations down, can
connect to it by path, and connections are independent. An inherited
`socketpair` is not used: it reaches only the descendants that keep the fd,
and it carries one connection. The executor serves one workspace root and one
snapshot. `cwd` is resolved inside that root, and two checkouts with the same
`sandbox.name` never share an executor (review W2, identity by root).

### E3. Snapshot, no composition

The executor's environment is the project snapshot frozen at `up` (provider
capture, then activation overlay, then `[env.vars]`, then `[env] forward`, in
the order `up` already applies). Each spawn is `env_clear` + snapshot + the
declared deltas (`TERM`, `COLUMNS`, `LINES` and a recipe-declared allowlist).
Nothing from the agent's environment crosses, by construction rather than by
a denylist of names (review R4).

### E4. Routing is per harness, and qualification is binary

A recipe in mode 2 declares how its native exec tool is disabled and how the
executor-backed tool is supplied. Two transports exist, and the recipe says
which:
- **MCP tool.** Piped only: argv or shell command in, stdout, stderr and exit
  status out, cancel by request. No PTY and no resize, since MCP has neither.
- **A harness lever** (a configurable shell or terminal backend), where one
  exists. It may carry a PTY.

The routing configuration must take effect without devcroft parsing ACP.
Some adapters take tool settings from the editor's `session/new` parameters
(the review cites `claude-agent-acp` reading `disallowedTools` and
`mcpServers` there). A recipe qualifies only if devcroft's settings file wins
over anything `session/new` can send, as measured. If the only way is to
rewrite `session/new`, the harness stays in mode 1, and a thin ACP shim
becomes a separate proposal with its own review. If the native path cannot be
disabled, the recipe is mode 1 only. `status` names the mode and the reason.

### E5. Two policies, two proxies, both rendered

`policy --render` shows the project profile and the agent profile, each rule
with its origin. Each tree has its own egress proxy, with its own allowlist,
token, socket, pidfile and log, and on Linux its own route-less network
namespace with its own relay:
- The agent profile and proxy hold the workspace (read-write), the agent's
  object and home (`agent:<recipe>`), the recipe's hosts and broker, and
  nothing of the project's network allowlist.
- The project profile and proxy are today's, minus the recipe's hosts and
  broker. The agent proxy's socket and token are not in its view, environment
  or logs.

### E6. Separate homes, explicit sandbox identity

Both trees share the workspace read-write, and the sandbox `HOME` is inside
it. In mode 2 the agent tree gets a `HOME` outside the workspace, in a
per-sandbox directory beside the artifact store (never under
`~/.local/share/devcroft`), granted to the agent profile only and removed at
`rm`. The project tree keeps `<project>/.devcroft/<name>/home`. Because
`own_sandbox_dir` infers the sandbox from `HOME`, each tree is given the
sandbox directory as an explicit variable, which `own_sandbox_dir` reads
first. The in-sandbox `agent` verbs and the denial hook then work from either
tree. A login stored by the agent therefore leaves the project's reach in
this mode.

### E7. Which tree each command reaches

- `exec`, `shell`, `ssh`, `ps` and services: the project tree, through the
  existing control socket.
- `agent acp` and `agent run`: the agent tree, through a second control
  socket the parent binds.
- `status`, `logs`, `policy --render`, `why`: both trees, labelled.
The mode 1 launcher is absent from the project tree in mode 2, because the
agent's object is not in the project profile. `devcroft shell` says so and
names `devcroft agent run`.

## Risks / Trade-offs

- [Authority union: the agent can run any `argv` through the executor] →
  stated in the proposal and the threat model; the separation claimed is the
  credential route, the runtime and the agent's state.
- [Harnesses change their tool configuration between versions] →
  qualification is per recipe version; an unqualified version falls back to
  mode 1 visibly.
- [MCP-routed execution has no PTY] → interactive commands through MCP fail
  as they would under any piped tool; a recipe needing a PTY qualifies only
  through a harness lever.
- [RPC latency, backpressure] → measured in group 0 (review L1).
- [A new process topology in `up`, and twice the proxies] → used only for
  sandboxes in mode 2. Mode 1 and agent-less sandboxes are byte-identical to
  today (asserted).

## Open Questions

- Reuse fleet's `Supervisor` for the `up` path, or a smaller parent?
- Should executor recovery after a crash be in the first slice (review S2)?
- On macOS, can two Seatbelt profiles share the workspace path with the same
  semantics as Linux bind mounts (review W1, W3)?

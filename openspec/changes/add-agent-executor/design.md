## Context

Source: the adversarial review of 2026-11-19 (family 4 in its comparison: two
sibling trees, an RPC `ProjectExecutor` and a private package). This change
takes that recommendation, on top of `add-agent-packages`. The review's
proposed acceptance matrix (R1–R7, W1–W4, L1, P1, S1–S2, U1, G1, N1, A1) is
the basis of group 0 and of the specs. The rows already covered by
`add-agent-packages` (P1, S1, U1, G1, A1) are not repeated.

Two facts verified on 2026-11-22 against the code:
- `SpawnRequest` (`src/keeper/protocol.rs`) carries `cmd`, `args`, `cwd`,
  `env` and `pty`, with no policy selector. The request's `env` overlays the
  keeper's.
- The proxy URL with its session token is inserted into the environment that
  every session inherits (`src/lifecycle/up.rs`, after the port allocations).

Four ACP exploration notes the review cites (`acp-agent-vm-separation-2026-11-16.md`
and siblings) are not present in any checkout. The "Form A / Form B" terms
are therefore not used here. "Mode 1" is `add-agent-packages` (one tree), and
"mode 2" is this change.

## Goals / Non-Goals

**Goals:**
- Separate policies and credential routes for agent and project, enforced by
  the OS on each tree.
- Every project operation a qualified harness emits through its exec tool
  runs in the project's snapshot, including direct `argv`, shell, PTY and
  their descendants.

**Non-Goals:**
- Intercepting every `execve`. A child the agent starts locally, bypassing its
  exec tool, stays in the agent tree and outside the guarantee (review R7).
- A second VM, or containment of hostile code.
- Filesystem RPC. The workspace is shared, so reads and writes stay local.

## Decisions

### E1. The parent is host-side and unrestricted; the trees are restricted

`up` gains a parent process, for this mode only, that creates the
executor's and the agent's listeners and fds, then forks two children. Each
child applies its own profile to itself, which keeps the keeper invariant that
a process restricts itself. The parent holds no restriction and serves no
session. It supervises, restarts and tears down. On Linux it mirrors fleet's
`Supervisor`, and the decision is whether to reuse it (open question). Mode 1
sandboxes keep today's single self-restricting keeper.

### E2. The executor is bound to one pair

The executor's socket is created by the parent and inherited by the agent tree
only. It is bound to one workspace root and one snapshot. `cwd` is resolved
inside that root, and two checkouts with the same `sandbox.name` never share
an executor (review W2, identity by root).

### E3. Snapshot, no composition

The executor's environment is the project snapshot frozen at `up` (provider
capture, then activation overlay, then `[env.vars]`, then `[env] forward`, in
the order `up` already applies). Each spawn is `env_clear` + snapshot + the
declared deltas (`TERM`, `COLUMNS`, `LINES` and a recipe-declared allowlist).
Nothing from the agent's environment crosses, by construction rather than by
a denylist of names (review R4).

### E4. Routing is per harness, and qualification is binary

A recipe in mode 2 declares how its native exec tool is disabled and how the
executor-backed tool is supplied. Group 0 measures each. If the native path
cannot be disabled, the recipe is mode 1 only, and `status` says so.

### E5. Two policies, both rendered

`policy --render` shows the project profile and the agent profile, each rule
with its origin. The agent profile holds the workspace (read-write), its
object and state (`agent:<recipe>`), its hosts and nothing of the project's
network allowlist. The executor profile is today's profile, minus the agent's
credential route.

## Risks / Trade-offs

- [Authority union: the agent can run any `argv` through the executor] →
  stated in the proposal and the threat model; the separation claimed is the
  credential route and the runtime.
- [Harnesses change their tool configuration between versions] →
  qualification is per recipe version; an unqualified version falls back to
  mode 1 visibly.
- [RPC latency, backpressure, PTY semantics] → measured in group 0 (review L1).
- [A new process topology in `up`] → used only for sandboxes in mode 2. Mode 1
  and agent-less sandboxes are byte-identical to today (asserted).

## Open Questions

- Reuse fleet's `Supervisor` for the `up` path, or a smaller parent?
- Should executor recovery after a crash be in the first slice (review S2)?
- On macOS, can two Seatbelt profiles share the workspace path with the same
  semantics as Linux bind mounts (review W1, W3)?

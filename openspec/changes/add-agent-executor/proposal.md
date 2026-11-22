# Change: add-agent-executor

**Depends on:** `add-agent-packages`. Not scheduled. It is the hardening of
that change, not a precondition for it.

## Why

`add-agent-packages` runs the agent in the project's sandbox, under one
policy. Commands the agent runs through its tools reach the project's
toolchain because they inherit the project's `PATH`. That is enough for the
Node 16 / Node 24 requirement and for accident protection. It leaves three
things open, and the adversarial review of 2026-11-19
(`new/adversarial-review-agent-runtime-workspace-separation-2026-11-19.md`)
names them:

- The agent and the project share one policy. The agent's vendor host is
  reachable from a project build script, and the project's registry access is
  reachable from the agent.
- The agent's credential route (proxy token and placeholder) is in the same
  session environment as the project's processes. The current code places
  the proxy URL with its token in the environment every session inherits.
- Nothing distinguishes "a command for the project" from "a process of the
  agent". A harness that runs tests by absolute path, or with an environment
  it built itself, escapes the project's context without anyone noticing.

The review's recommendation addresses all three: one supervisor, two process
trees with separate policies, and a `ProjectExecutor` reached over RPC.

## What Changes

- **Two trees, one supervisor.** For a sandbox with an agent, an unrestricted
  host-side parent creates every listener first, then restricts and starts
  two children: the project executor, under the project's policy and
  environment snapshot, and the agent, under an agent policy (its object, its
  state, the workspace, its hosts). Neither tree can widen the other. This
  changes `up`: today the keeper restricts itself and there is no
  unrestricted parent. Fleet already has one (`Supervisor`). The
  listener-before-restriction invariant is kept per tree.
- **`ProjectExecutor` RPC.** The only path from agent to project. It accepts
  `argv`, an explicit shell flag, a `cwd` relative to the workspace, stdio or
  a PTY with resize, signals and cancel, and a small declared set of
  environment deltas. It never accepts a root, a mount, a grant, a network
  policy or an environment snapshot from the agent. Every spawn is `env_clear`
  plus the project snapshot captured at `up`.
- **Per-harness routing.** For each qualified recipe and version, devcroft
  generates the harness configuration that disables its native exec tool and
  supplies an exec tool backed by the executor, through MCP or the harness's
  own lever. Read, write and search stay local on the shared workspace.
  Support is declared per recipe version × operation × `up`/fleet × platform.
  A harness whose native exec path cannot be turned off is not qualified for
  this mode, and keeps the `add-agent-packages` mode.
- **Credential route only in the agent tree.** The proxy token, placeholder
  and socket are absent from the executor's environment, filesystem and logs.
- **Stated limit.** The executor runs any `argv` in the project's context, so
  the agent's effective authority is the union of both policies. What
  separates is the credential route and the runtime, not reachability. The
  process tier stays accident protection.

## Capabilities

### New Capabilities

- `project-executor`: the RPC, the snapshot, the pair binding and the
  per-harness routing.

### Modified Capabilities

- `lifecycle`: an unrestricted parent and two restricted trees for a sandbox
  with an agent in this mode.
- `policy`: two compiled policies per such sandbox, both rendered, with
  origins.

## Impact

- `src/lifecycle/up.rs` (topology), a new `src/executor/`,
  `src/keeper/{protocol,session,connection}.rs` (`SpawnRequest` is reused
  for the executor's spawns but not exposed to the agent as-is: today it
  accepts `cwd` and `env` from the caller, which is a spawn authority, not a
  transport), `src/policy` (render two profiles), `src/agent/` (routing
  configuration per recipe).
- `docs/threat-model.md`: what the second tree adds (credential route,
  runtime) and what it does not (reachability, containment).
- ACP is the editor's transport and is unchanged. Per the review, ACP v1's
  client-owned `terminal/*` and `fs/*` are optional and v2 drafts remove them,
  so they are not the routing mechanism. That is the review's reading at
  pinned revisions, not re-verified here.

See [design.md](design.md).

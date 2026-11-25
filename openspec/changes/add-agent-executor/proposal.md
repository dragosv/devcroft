# Change: add-agent-executor

**Depends on:** `add-agent-packages`, and on macOS `add-macos-unix-socket-scoping`
verified live. Not scheduled. It is the hardening of `add-agent-packages`
("mode 2"), not a precondition for it.

## Why

`add-agent-packages` ("mode 1") runs the agent in the project's sandbox, under
one policy. Commands the agent runs through its tools reach the project's
toolchain because they inherit the project's `PATH`. That is enough for the
Node 16 / Node 24 requirement and for accident protection. It leaves three
things open:

- The agent and the project share one policy. The agent's vendor and login
  hosts are reachable from a project build script, and the project's registry
  access is reachable from the agent.
- The agent's credentials and credential route are shared with the project's
  processes. In mode 1 there is one egress proxy with one allowlist, one
  sandbox environment and one `HOME`: every session sees a broker's endpoint
  and placeholder (or a delivered `auth = "env"` variable), can reach the
  proxy's socket, and can read a subscription token stored under `auth =
  "login"` in `HOME`.
- Nothing distinguishes "a command for the project" from "a process of the
  agent". A harness that runs tests by absolute path, or with an environment
  it built itself, escapes the project's context without anyone noticing.

Addressing all three requires one supervisor, two process trees with separate
policies, homes and egress proxies, a `ProjectExecutor` reached over RPC, and
explicit boundary checks for policy overlaps, `[env] forward` collisions,
socket mediation on macOS, and Mode 1 → Mode 2 credential transitions.

## What Changes

- **Opt-in, never automatic.** `[agent] mode = 2` for new sandboxes or `up
  --agent-mode 2`, recorded with the selection. The default stays mode 1, and
  upgrading devcroft never changes a sandbox's mode. A sandbox that asks for
  mode 2 and is not eligible fails at layer `config` naming every unmet
  condition; it is never downgraded.
- **Two trees, one supervisor.** An unrestricted host-side parent creates every
  listener first, then restricts and starts two children: the project executor,
  under the project's policy and environment snapshot, and the agent, under an
  agent policy (its object, its separate `HOME`, the shared workspace, its
  active hosts). Eligibility needs `network.default = "deny"`, a boundary that
  hides the other tree's unix sockets and proxy port (on Linux a constructed
  mount view and network namespace, since Landlock does not mediate AF_UNIX; on
  macOS verified Seatbelt socket and `proxy_only` denials), and, whenever the
  selected auth mode places a real upstream credential in the agent's
  environment (`auth = "env"`), a boundary that hides the other tree's process
  environment (`auth = "env"` is therefore ineligible on macOS until measured
  closed, while `api-key` and `login` remain eligible once Seatbelt port/socket
  denials pass). Mode 1 and agent-less sandboxes keep today's single
  self-restricting keeper byte-identically.
- **`ProjectExecutor` RPC and collision-free project snapshot.** The RPC is the
  only path from agent to project. It accepts `argv`, an explicit shell flag, a
  `cwd` relative to the workspace, stdio or a PTY with resize, signals and
  cancel, and a small declared set of environment deltas — never a root, mount,
  grant, policy or environment snapshot. Every spawn is `env_clear` plus the
  project snapshot captured at `up`. Before provisioning, `up` refuses at layer
  `config` any collision between `manifest.env.vars`, `manifest.env.forward` or
  `[credentials] env` and the recipe's broker variables, `auth = "env"` key,
  `[launch.env]` or `reserved_env`, so `apply_manifest_env` can never copy the
  agent's secret into the project snapshot.
- **Per-harness routing with `O_NOFOLLOW` `HOME` merge.** For each qualified
  recipe and version, devcroft merges into the agent's separate `HOME` (via
  `OwnedDir`'s `O_NOFOLLOW` descriptor-anchored writes) the configuration that
  disables its native exec tool and supplies an executor-backed tool through
  MCP (piped only) or a harness lever, without clobbering user settings or
  stored logins. A harness whose native exec path cannot be disabled without
  rewriting ACP is not offered in mode 2.
- **Two egress proxies, read-overlap checks and no silent handling of mode 1
  state.** Each tree gets its own egress proxy, allowlist, token, socket and
  log, and on Linux its own network namespace. Canonical resolved grants in a
  project profile (`Read` as well as `ReadWrite`) are checked before startup
  against `~/.local/share/devcroft-agent-homes` and the agent's socket
  directory. Mode 2 isolates `HOME` per sandbox **and** recipe
  (`devcroft-agent-homes/<sandbox-id>/<recipe>/home`), so switching recipes in
  mode 2 never exposes one agent's stored login to another. Whenever mode 2 is
  requested for a recipe that has ever run in mode 1 on that sandbox
  (`Meta.mode1_recipes`, even across an intermediate `--no-agent` or recipe
  switch) and its project `HOME` holds the recipe's `state_paths`, `up` refuses
  and lists them; `--quarantine-agent-state` moves them, undeleted, to a
  directory neither tree is granted. They are never copied into the agent's
  `HOME`, and `up` says that a login used in mode 1 was readable by the project
  and should be revoked.
- **Separate homes and OSC 7501 preservation.** Sandbox identity is passed via
  an explicit variable rather than inferred from `HOME`. `devcroft agent run`
  connects to the agent tree's control socket over a PTY, preserving full
  bidirectional **OSC 7501** (`herdr-core`) terminal transparency, while `agent
  acp` and MCP-routed `ProjectExecutor` calls stay piped and free of OSC
  traffic.
- **Stated limits, both directions.** *Agent to project:* the executor runs any
  `argv` in the project's context, so the agent's effective authority is the
  union of both policies. *Project to agent:* the workspace is shared and
  writable, so project code can write files that the agent's harness runs or
  obeys in the agent tree (project-level settings and hooks, MCP declarations,
  instruction files, git hooks). Mode 2 therefore removes *direct* access to the
  agent's credentials and route, not influence over a process that holds them.
  The process tier stays accident protection.

## Capabilities

### New Capabilities

- `project-executor`: the RPC, the collision-free snapshot, socket exclusivity
  (including `network.default = "deny"` eligibility) and per-harness routing.

### Modified Capabilities

- `lifecycle`: unrestricted parent, two restricted trees, separate homes, the
  mode 1 to mode 2 transition, and OSC 7501 over the agent tree's PTY socket.
- `config`, `cli`: `[agent] mode`, `up --agent-mode`,
  `--quarantine-agent-state`.
- `policy`: two compiled policies per mode 2 sandbox, canonical read/write
  overlap rejection on the agent `HOME` and socket roots, and isolation of all
  agent auth modes to the agent tree.

## Impact

- `src/lifecycle/up.rs` (topology, two egress proxies, `[env] forward`
  collision checks, mode 1 to mode 2 transition), `src/proxy`,
  `src/policy/capability_set.rs` (`agent_home_overlap` checking both `Read` and
  `ReadWrite` on canonical paths; Seatbelt socket denials on macOS),
  `src/runtime/owned_dir.rs`, `src/bin/devcroft.rs` (`own_sandbox_dir`,
  innermost-root discovery, valid `up [--name <n>]` remedies, command-to-tree
  mapping), `src/lifecycle/terminate.rs`, `src/executor/`, `src/agent/`.
- `docs/threat-model.md` and `docs/agents.md`: what mode 2 separates, why a
  mode 1 login should be revoked, and what each tree can still do to the other.

See [design.md](design.md).

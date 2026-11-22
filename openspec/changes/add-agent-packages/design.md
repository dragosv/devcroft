## Context

Requirement (owner, 2026-11-19, `new/agent-runtime-workspace-separation-2026-11-19.md`):
an agent with its ACP adapter must be easy to add and must not depend on the
project's provider. The adversarial review of that note
(`new/adversarial-review-agent-runtime-workspace-separation-2026-11-19.md`)
recommends two process trees and an RPC executor. That is the strong form,
and it is split into `add-agent-executor`. This change is the form that
delivers the requirement inside today's keeper model, where one sandbox runs
under one policy.

What exists and is reused:

| Piece | Where | Reused for |
|---|---|---|
| Content-addressed read-only store, leases, sweep, completion record | `src/provider/artifacts.rs` | agent objects |
| Header walk for host libraries | `artifacts::header_dependencies` | host runtime of a Linux/macOS Node or a native binary |
| Confined provisioning phase with its own network | `sandbox-provisioning` (`__provision`) | recipes whose install must run code |
| Credential broker | `add-agent-workload` 4b | auth, unchanged |
| Piped session with separate stdout/stderr | `devcroft exec` | the ACP stdio bridge |
| `agent` command group | `src/bin/devcroft.rs` (`agent hook claude-code`) | `agent acp`, `agent list` |

## Goals / Non-Goals

**Goals:**
- From a clean host to an agent answering ACP in a sandbox, with one line of
  user configuration and no agent preinstalled.
- The agent's runtime never changes what the project's commands resolve.
- Every agent artifact verified, recorded and visible in `status` and
  `policy --render`.

**Non-Goals:**
- Separate policies for agent and project, routed execution, or
  credential-route separation between them (`add-agent-executor`).
- Containment of a hostile agent. One process tier, accident protection
  (`docs/threat-model.md`).
- Arbitrary user-authored recipes in the first slice (an open question below).
- A second VM.

## Decisions

### A1. Recipes ship with devcroft

A recipe is data compiled into the binary: one TOML file per agent under
`src/agent/recipes/`. A devcroft release fixes the recipe set, and the recipe
version is part of the object id. *Alternatives considered:*
- A remote catalog, fetched at `up`. Rejected for the first slice: it is
  another resolver at provisioning time, and a recipe is code-adjacent
  (entrypoint, argv, network hosts).
- The ACP registry alone. Rejected as the source of truth: it provides
  distribution metadata, not proof that a package is complete and starts
  without further downloads (the note's §6.2). It may seed recipes.

### A2. Recipe contents

```toml
name = "claude"                     # what the user writes
version = "…"                       # recipe version, in the object id
acp = "adapter"                     # "native" | "adapter"

[[artifact]]                        # one per component per platform
role = "runtime"                    # runtime | harness | adapter
platform = "linux-aarch64"
url = "https://…"
sha256 = "…"
layout = "node-tarball"             # node-tarball | single-binary | npm-lock | archive

[launch]
acp = ["{runtime}/bin/node", "{adapter}/dist/index.js"]
interactive = ["{harness}/bin/claude"]   # what the launcher execs
command = "claude"                  # the launcher's name on PATH

[launch.env]                        # absolute paths for everything the chain starts
CLAUDE_CODE_EXECUTABLE = "{harness}/bin/claude"   # illustrative; gate measures the real lever
CLAUDE_CONFIG_DIR = "{state}/config"

[network]
hosts = ["api.anthropic.com"]

[auth]
methods = ["api-key-broker"]
```

The variable names above are illustrative. Group 0 measures each harness's
real levers.

### A3. Install executes nothing downloaded, by layout kind

- `single-binary`, `archive`, `node-tarball`: verify, then extract. Nothing
  runs.
- `npm-lock`: the recipe pins a `package-lock.json` (lockfile v3). devcroft
  lays out `node_modules/` itself: for each lock entry it takes the install
  path, `resolved` and `integrity`, verifies the tarball and extracts it to
  that path. It selects platform-specific optional packages by the lock's
  `os`/`cpu` fields and runs no lifecycle script. An entry with
  `hasInstallScript: true` refuses the recipe unless the recipe states, with
  measured evidence, that the script is not needed.
- A recipe that needs code at install runs it in confined provisioning
  (`__provision`), writing only into the staging object, with network limited
  to the recipe's install hosts. It is then marked as such in `status`.

*Alternative considered:* running a pinned `npm ci --ignore-scripts`
host-side. Rejected because it executes npm, which is downloaded code. The
same rule refused mise's core backends.

### A4. Same sandbox, private runtime, project environment

The agent process starts through the keeper like any session. Its
environment is the project's environment, plus the recipe's `[launch.env]`,
plus the broker placeholder. Its argv is absolute into the object. Nothing
from the object is prepended to `PATH`. Consequences:
- The agent's Bash-type tool spawns through the inherited `PATH`, so it gets
  the project's `node`, `npm` and libraries. This is the Node 16 / Node 24
  property for every command that resolves through `PATH`.
- `child_process.fork()` uses `process.execPath`, so the agent's own workers
  stay on its runtime. That is correct, not a leak.
- An adapter that starts the harness through `PATH` would pick the project's
  `node`. `[launch.env]` must pin every such executable. Group 0 detects
  misses with a sentinel `node` on the project's `PATH`.

### A5. Launcher, after the project

`<state>/<name>/agent/bin/<command>` is a small script that execs the recipe's
`interactive` argv. That directory goes on `PATH` after the project's entries
and after the tooling layer, and a shadowed launcher is reported the way
`tooling::shadowed` reports it. Only the agent's own command name is on
`PATH`, never its runtime.

### A6. State

Writable per sandbox and per recipe: `<state>/<name>/agent/<recipe>/`, holding
config, history and cache, reached through the recipe's state variables, not
by moving `HOME`. Moving `HOME` would move it for the project's commands
too. It is removed at `rm`, and it is outside the object, the workspace and
the project's caches.

### A7. ACP endpoint

`devcroft agent acp [name]` auto-starts the sandbox if needed, as `exec`
does. It then runs the recipe's `acp` argv as a piped session and passes
stdin/stdout through untouched, with stderr kept separate. devcroft does not
parse ACP. The editor's configuration is `devcroft agent acp <name>`.

### A8. Tier and policy

The object, and on Linux the host libraries its runtime needs (header walk),
are read-only grants with origin `agent:<recipe>`. The state directory is a
write grant with the same origin. The recipe's hosts join the egress
allowlist with that origin too. A manifest write grant overlapping the
artifact store is already refused at `config`. `status` prints "agent:
<recipe> <version> (artifact tier)".

## Risks / Trade-offs

- [The agent shares the project's policy and network] → stated in `status`
  and docs; `add-agent-executor` is the remedy.
- [Harness chains look executables up on `PATH`] → A4's sentinel test is a
  qualification criterion per recipe; a miss blocks the recipe.
- [Recipes age as vendors ship weekly] → recipe updates are devcroft
  releases. An open question asks whether a signed recipe channel should
  exist.
- [Vendor self-update writes into the install] → the object is read-only.
  The recipe sets the vendor's update-disable switch, and a failed update is
  visible, not silent.

## Open Questions

- Should user-authored recipes (a file in the user configuration) be allowed,
  and under which checks?
- Should a project be able to declare `[agent]` for a team, beside the user's?
- Should the first recipes be one native-ACP binary plus one npm adapter
  (Claude through its adapter), or the binary alone?

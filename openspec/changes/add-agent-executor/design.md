## Context

This change ("mode 2") builds two sibling restricted process trees and an RPC
`ProjectExecutor` on top of `add-agent-packages` ("mode 1"). Where mode 1
delivers private packages, `reserved_env` stripping, install pins, innermost-root
discovery and bidirectional **OSC 7501** inside a single keeper, mode 2
delivers state and credential separation (`S1`), environment/runtime snapshot
enforcement (`R2`–`R6`) and network/route separation (`N1`).

Codebase facts that constrain mode 2 (verified directly against `src/` and
active specs):
- `SpawnRequest` (`src/keeper/protocol.rs`) carries `cmd`, `args`, `cwd`,
  `env` and `pty`, with no policy selector.
- `apply_manifest_env` (`src/lifecycle/up.rs:3156`) copies every variable in
  `manifest.env.forward` from the host shell into the project's captured
  environment. Without an explicit pre-provisioning collision refusal covering
  both `api-key` and `auth = "env"` variables, `env_clear` + restoring the
  project snapshot would place the agent's real secret straight back into every
  project process.
- `artifact_store_write_overlap` (`src/policy/capability_set.rs:128`) checks
  `AccessMode::ReadWrite` only. `check_no_deny_overlaps_allow` already refuses a
  grant of `~/.local/share` or `~` as written, because devcroft's denied data
  directory lies inside it, but it compares strings: it does not see
  `~/.local/share/devcroft-agent-homes` itself or a symlink that resolves to it.
- The workspace is writable by both trees, and harnesses read project-level
  configuration from it.
- macOS has no process visibility separation between sandboxes
  (`docs/known-gaps.md`), so whether one tree can read the other's environment
  via `KERN_PROCARGS2` / `ps -E` is not established there. Moreover, on macOS
  (`isolate_network == false`), `HTTPS_PROXY` in the agent tree carries
  `http://nono:<token>@127.0.0.1:<agent_port>` (`src/proxy/mod.rs:344`) and
  `api-key` carries `broker_phantom`: neither is a real upstream secret, and
  Seatbelt's `proxy_only(project_port)` (`src/policy/capability_set.rs:366`)
  blocks the project tree from connecting to `<agent_port>`, whereas `auth =
  "env"` places a **real** upstream credential directly in the agent's
  environment.
- `allow-landlock-only-fallback` proposes to run without a mount view; Landlock
  alone does not hide a unix socket.
- On macOS, Seatbelt has no mount namespace; pathname unix-socket `connect()` is
  mediated on the `network-outbound` axis (`add-macos-unix-socket-scoping`,
  `specs/macos-unix-socket-mediation/spec.md:28-34`). When
  `network.default = "allow"` (permitted by `auth = "login"` and `auth =
  "env"`), `network-outbound` is open, so omitting a positive socket grant from
  the project profile does **not** block `connect()` to the executor's unix
  socket.
- In mode 1, `<project>/.devcroft/<name>/home` holds the agent's login token
  across `down`, `up` and `up --recreate`. In mode 2, the project tree keeps
  that exact directory as its `HOME`. Leaving a mode 1 token there, or copying
  it, breaks the isolation claim. But the same files hold history and settings,
  and a `claude` from the project's environment or `[tools]` writes there
  legitimately, so they cannot be deleted on sight either.
- macOS has no `PR_SET_PDEATHSIG`; teardown cannot rely on the parent process
  surviving `SIGKILL`.

## Goals / Non-Goals

**Goals:**
- Separate policies, egress proxies, `HOME` directories and credential custody
  (`api-key` broker, `login` tokens and `env` keys) for agent and project,
  verified against overlapping manifest grants, `[env] forward` collisions and
  Mode 1 → Mode 2 transitions.
- Every project operation a qualified harness emits through its exec tool runs
  in the project's snapshot, including direct `argv`, shell and descendants.
- Preserve mode 1's UX and safety invariants: `O_NOFOLLOW` descriptor-anchored
  `HOME` merges, the selection rule, checks before teardown, innermost-root
  discovery, bounded `agent acp` auto-up, ACP session lifecycle, and
  bidirectional **OSC 7501** transparency on `agent run`.

**Non-Goals:**
- Intercepting every `execve`. A child the agent starts locally, bypassing its
  exec tool, stays in the agent tree and outside the guarantee (review R7).
- Offering mode 2 on projects with `network.default = "allow"`.
- Stopping project code from influencing the agent through the workspace.
- A second VM, filesystem RPC, parsing ACP, or containment of hostile code.

## Decisions

### E0. Mode 2 is asked for, and refused rather than downgraded

`[agent] mode` (1 or 2, default 1) sets the mode of new sandboxes; `up
--agent-mode <1|2>` chooses explicitly. The mode is part of the recorded
selection and follows `add-agent-packages` A7: kept across `down`, `up`,
`--recreate` **and recipe switches** (unless `--agent-mode` or
`--agent-from-config` changes it), changed on an existing sandbox only with
`--recreate`, and never changed by an upgrade or by editing `[agent]`. Switching
a mode 2 sandbox to a recipe or auth mode that is not eligible for mode 2
therefore fails before teardown naming `--agent-mode 1`, rather than silently
downgrading to mode 1.

Mode 2 is eligible when all of these hold, and `up` otherwise fails at layer
`config` (or `backend`, for a host condition) before teardown, naming each:
- `network.default = "deny"`;
- Linux: the mount view and the network namespace can be constructed. A sandbox
  running under any Landlock-only fallback is not eligible;
- macOS: task 0.5 verified the Seatbelt socket and `proxy_only` port denials on
  that release;
- whenever the selected auth mode places a real upstream credential in the
  agent's environment (`auth = "env"`), the other tree's process environment is
  unreadable (on Linux by a PID namespace and private `/proc` per tree; on macOS
  as task 0.4 measures `KERN_PROCARGS2` / `ps -E` — until measured closed,
  `auth = "env"` is not eligible in mode 2 on macOS, while `api-key` and `login`
  remain eligible once Seatbelt blocks connecting to `<agent_port>`);
- the recipe version is qualified for mode 2 (E4).
`status` shows the mode.

### E1. Unrestricted host-side parent; restricted sibling trees

`up` starts an unrestricted host-side parent, for mode 2 only, that creates
every listener, socket and fd of both trees, then forks two children that each
self-restrict. The parent serves no session. Mode 1 and agent-less sandboxes
keep today's single self-restricting keeper byte-identically.

Every pid of both trees and both proxies is recorded in the state directory as
it starts. `down` and the next `up`'s orphan recovery stop each recorded
process group so a `SIGKILL`ed parent on macOS or Linux leaves nothing behind.

### E2. Executor socket exclusivity

A project with `network.default = "allow"` has no outbound boundary and leaves
Seatbelt's `network-outbound` axis open on macOS, so it is not eligible (E0).

Under `network.default = "deny"`, the parent binds the executor's pathname unix
socket inside a dedicated `0700` agent-sockets directory before restriction:
- **Linux:** mounted into the agent tree's filesystem view alone; absent from
  the project tree's view (`ENOENT`).
- **macOS:** gated on live hardware verification of `add-macos-unix-socket-scoping`
  (Group 0 task 0.5). Only the agent Seatbelt profile receives the scoped
  `network-outbound` / `file-read*` grant for the executor and agent-proxy
  sockets, and the project Seatbelt profile also emits explicit trailing
  `(deny file-read* file-write* (subpath ...))` and
  `(deny network-outbound (literal ...))` rules on the agent-sockets directory
  after all manifest grants.

The executor serves one canonical `project_root` and one snapshot; `cwd`
outside that root is refused, and two checkouts with the same `sandbox.name`
never share an executor.

### E3. Collision-free project snapshot

Before provisioning, at layer `config`, mode 2 rejects `up` if
`manifest.env.vars`, `manifest.env.forward` (`src/lifecycle/up.rs:3156`) or
user `[credentials] env` names any credential or control variable of the
selected recipe: the broker's `env` or `endpoint_env`, the `auth = "env"` key,
any `[launch.env]` key, or any `reserved_env` key. Consequently
`apply_manifest_env` never copies an agent secret into the project snapshot.

Each executor spawn runs `env_clear` + the frozen `up` snapshot + declared
deltas (`TERM`, `COLUMNS`, `LINES` and a recipe-declared allowlist). Nothing
from the agent tree's environment crosses into the project tree. Unlike mode 1,
where the broker's placeholder is written last, the project tree has no broker
entry that would overwrite a forwarded key, so here the collision is a real leak.

### E4. Per-harness routing, `O_NOFOLLOW` merge, and binary qualification

A mode 2 recipe version declares how its native exec tool is disabled and how
the executor-backed tool is supplied (piped MCP with zero OSC traffic, or a
harness lever that may carry a PTY). Routing files are written into the agent
tree's separate `HOME` through `add-agent-packages` A6's `O_NOFOLLOW`
descriptor-anchored, key-scoped merge (`OwnedDir`), preserving user settings
and stored `auth = "login"` tokens across `down` and `up`, and are folded into
`recipe_digest`. A recipe version qualifies only if devcroft's configuration
cannot be overridden by an editor's `session/new` parameters without parsing
ACP; otherwise it is not offered in mode 2.

Qualification also lists every file in the workspace that the harness runs or
obeys (settings, hooks, MCP declarations, instructions), and any lever that
turns project-level configuration off. Where a lever exists the recipe uses it;
where none does, the recipe's entry says so. This narrows the project-to-agent
direction and does not close it.

### E5. Two policies, two proxies, and canonical read/write overlap rejection

`policy --render` renders both the project profile and the agent profile with
origins. Each tree owns a separate egress proxy, allowlist, token, socket,
pidfile and log (and on Linux its own route-less network namespace and relay):
- **Agent profile and proxy:** workspace (read-write), agent `<object>` and
  separate agent `HOME` (`agent:<recipe>`), active recipe hosts (`[network].hosts`
  plus `[auth.login].hosts` under `auth = "login"`), and the active auth
  mechanism (`api-key` broker, `login` tokens in agent `HOME`, or `auth = "env"`
  variable). None of the project's `network.allow` hosts.
- **Project profile and proxy:** today's project rules, with zero `agent:`
  rules, hosts, brokers or `auth = "env"` variables.
- **Canonical read + write overlap check (`agent_home_overlap`):** beside
  `artifact_store_write_overlap` (`src/policy/capability_set.rs:128`),
  `capability_set` checks **every** project profile's canonical resolved grants
  (`AccessMode::Read` **and** `AccessMode::ReadWrite`, in mode 2, mode 1 and
  agent-less sandboxes alike, since any of them could read another's agent
  `HOME`) against `~/.local/share/devcroft-agent-homes`
  and the agent-sockets root. Any grant that contains or is contained by either
  root (a direct grant or a symlink alias; the lexical parents already fail today) fails
  `up` at layer `config` naming the grant before any process starts. A sandbox already running when
  the first mode 2 sandbox starts was compiled under the same check, so it has
  no such grant either; one started by an older devcroft is refused by name
  until it is recreated. The profile's bytes do not change.

### E6. Separate per-recipe homes and Mode 1 → Mode 2 quarantine

Mode 2 places the agent tree's `HOME` at
`~/.local/share/devcroft-agent-homes/<sandbox-id>/<recipe>/home` (outside the
workspace and outside `~/.local/share/devcroft`), granted to the active recipe's
agent profile only and removed (under `<sandbox-id>/`) at `rm`. Keying by
`<recipe>` as well as `<sandbox-id>` ensures that switching recipes in mode 2
never exposes a previous recipe's saved login token to the new recipe. The
project tree keeps `<project>/.devcroft/<name>/home`. Both trees receive the
sandbox identity via an explicit environment variable read first by
`own_sandbox_dir`.

**Quarantine of prior Mode 1 state.** `Meta` maintains `mode1_recipes`
(`add-agent-packages` A7): every recipe that has ever run in mode 1 on this
sandbox, preserved across `--no-agent` and recipe switches until quarantined or
removed at `rm`. Whenever mode 2 is requested for recipe `R` and `R` is in
`Meta.mode1_recipes`:
1. If any of `R`'s `state_paths` exists in `<project>/.devcroft/<name>/home`,
   `up` fails at layer `config`, before teardown, listing them and naming
   `--quarantine-agent-state` (even if an intermediate `up --no-agent --recreate`
   or recipe switch occurred since `R` last ran in mode 1). Nothing is touched.
2. With `--quarantine-agent-state`, after any running keeper has stopped, those
   paths are moved through `O_NOFOLLOW` descriptors to
   `~/.local/share/devcroft-agent-homes/<sandbox-id>/quarantine/<recipe>-<time>/`,
   which no profile grants, and `R` is removed from `Meta.mode1_recipes` once
   all of `R`'s `state_paths` are out of the project `HOME`. Across filesystems
   the move is a descriptor-anchored copy, fsync and removal of the source only
   once the copy is complete. A symlinked component is refused. A failed move
   fails `up` and leaves the source and `mode1_recipes` intact.
3. Quarantined files are never copied into Mode 2's agent `HOME`: mode 1
   processes could read them. `up` prints where they are, that a login used in
   mode 1 was readable by the project and should be revoked with the vendor, and
   that a new login is needed. `rm` removes `<sandbox-id>/` (including
   `quarantine/`) and says so first.

When `R` has never run in mode 1 on this sandbox (`R` not in `mode1_recipes`),
`state_paths` in `<project>/.devcroft/<name>/home` are left alone: they belong
to whatever the project or `[tools]` runs. This preserves `add-agent-packages`
A6: only `rm` deletes state, and only `--quarantine-agent-state` moves it.

### E7. Command-to-tree mapping and OSC 7501 transparency

- `exec`, `shell`, `ssh`, `ps` and services reach the **project tree**
  (`devcroft shell` and `devcroft ssh` retain bidirectional OSC 7501 support).
- `agent acp` and `agent run` reach the **agent tree** through its dedicated
  control socket using `add-agent-packages` A8's innermost-root discovery,
  bounded `ACP_AUTO_UP_DEADLINE`, and valid `up [--name <n>]` remedies.
- `agent run` shares `src/exec.rs`'s checked raw-mode PTY
  client (`add-agent-packages` A14), guaranteeing identical bidirectional
  **OSC 7501** (`herdr-core`) negotiation and live state reporting in mode 2
  while `agent acp` and MCP-routed executor calls stay piped and OSC-free.
- `status`, `logs`, `policy --render` and `why` report both trees, labelled.

## Risks / Trade-offs

- [Authority union: the agent can run any `argv` through the executor] →
  stated in proposal, `policy --render` and `docs/threat-model.md`.
- [Project code writes a file that the agent runs or obeys] → stated in the
  proposal, `policy --render` and `docs/threat-model.md`; narrowed per recipe
  where a lever exists (E4).
- [Mode 1 to mode 2 needs a new login and an explicit flag] → honesty over a
  false sense of retroactive isolation, and no data lost silently.
- [Ineligible projects cannot use mode 2] → refused by name when asked for.
- [macOS socket isolation depends on `add-macos-unix-socket-scoping` measurement]
  → gated on hardware verification in Task 0.5 before mode 2 is enabled on
  macOS.

## Open Questions

- Reuse fleet's `Supervisor` for the `up` path, or a smaller parent?
- Should executor crash recovery be in the first slice (review S2)?

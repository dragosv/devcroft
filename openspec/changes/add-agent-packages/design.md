## Context

Adding a coding agent — over ACP from an editor or interactively in a terminal
— must not depend on the project's provider and must not require `mise`, `npm`
or `nix` on the host. In addition, bidirectional **OSC 7501** (`herdr-core` /
`hc ls`) terminal negotiation and live state reporting is mandatory across
every supported interactive PTY path (`agent run`, `devcroft shell`, `devcroft
ssh`), while `agent acp` stays strictly clean JSON-RPC stdio.

The two-tree RPC architecture is split into `add-agent-executor` ("mode 2").
This change ("mode 1") delivers agent packages inside today's single-keeper
sandbox. Reviews against the codebase surfaced the following concrete facts and
constraints, restated here so this specification stands on its own without
untracked notes:

- `<state>/<name>/` is under `~/.local/share/devcroft`, which is a baseline
  deny; `why_path` lets a deny win over any allow. Nothing of the agent's may
  live there.
- `check_brokers` (`src/lifecycle/up.rs`) compiles the manifest alone and
  checks `manifest.env.vars` for collisions, not `manifest.env.forward`. This
  is an ambiguity and **not a leak through the broker**: `up_process` inserts
  the broker's endpoint and placeholder last, after `apply_manifest_env`, so
  the forwarded value is overwritten (and the activation script gets the same
  environment). The leak is in the other modes: with no broker, `forward`
  delivers the real key, so an `auth = "login"` agent can silently use an API
  key instead of the subscription.
- `up --recreate` terminates the running keeper before the provider resolves,
  and `check_brokers` runs after that resolution. A check that fails there has
  already cost the user a working sandbox. A healthy sandbox returns
  `AlreadyUp` before any selection or credential is read.
- `SpawnRequest` (`src/keeper/protocol.rs`) can only add variables, and the
  keeper applies the activation overlay before the request's `env`. Nothing
  today can unset an inherited variable for one session.
- `maybe_auto_up` starts with `UpOptions::default()`, ignoring the user
  configuration.
- `<project>/.devcroft/<name>/` is write-denied from inside except `home` and
  the run directory (`with_read_only_artifacts`, `add-sandbox-identity` D4),
  **but `with_read_only_artifacts` adds no read grant** (`src/policy/mod.rs:636`):
  any subdirectory inside `.devcroft/<name>/` that must work when the manifest
  does not grant `.` needs its own `filesystem_read` grant, as `runtime/` does.
- On macOS there is no network namespace (`isolate_network == false`), so
  `HTTPS_PROXY` carries `http://nono:<token>@127.0.0.1:<port>` (`src/proxy/mod.rs:344`).
  Brokered `api-key` requests go to `http://127.0.0.1:<port>/broker<n>` under
  `NO_PROXY` without userinfo, whereas `login` and non-brokered `CONNECT`
  traffic hit `HTTPS_PROXY` userinfo (an unauthenticated TCP relay on macOS's
  shared loopback would be an open relay; `docs/agents.md` has not yet measured
  which harnesses handle `nono:<token>@` on macOS).
- `up`, `down` and `rm` serialize on one per-sandbox lifecycle lock.
- When `[auth.api_key_broker]` is unconditionally active, `check_brokers`
  refuses `up` whenever the API key is absent from the host shell — blocking
  subscription (`login`) users and non-brokerable agents such as `cursor`
  (`docs/agents.md`: ignores `NO_PROXY`, sends `CONNECT` to loopback, exchanges
  its key for a session token).
- `@agentclientprotocol/claude-agent-acp` (`v0.110.1`) bundles its own paired
  Claude runtime through `@anthropic-ai/claude-agent-sdk` (`0.3.330`). Upstream
  warns that setting `CLAUDE_CODE_EXECUTABLE` switches to a code path that
  passes session MCP environment and headers in child `argv` (visible in
  `/proc/<pid>/cmdline`), whereas the bundled runtime receives them over
  `stdin`. Furthermore, in mode 1 the adapter inherits the sandbox environment,
  so a `CLAUDE_CODE_EXECUTABLE` already set by the project's environment would
  still trigger that path unless explicitly stripped at agent launch.
- Sandbox `HOME` (`<project>/.devcroft/<name>/home`) is writable from inside
  the sandbox. As measured for `src/runtime/owned_dir.rs`, any host-side read
  or write by plain path into a sandbox-writable directory can follow a planted
  symlink onto arbitrary host files unless anchored with `O_NOFOLLOW` and
  atomic `renameat`.
- Resolving a sandbox from recorded `Meta` entries before checking upward
  `devcroft.toml` roots causes a fresh nested subproject (`/repo/sub/devcroft.toml`
  with no `Meta` yet) inside an already-running parent project (`/repo`) to
  attach to the outer parent sandbox's policy and toolchain.
- Even when an agent object is pre-installed, synchronous `up` inside `agent
  acp` can exceed an editor's `initialize` timeout if the project's own
  provider closure has never been built.
- `devcroft up` (`src/bin/devcroft.rs:2758,4861`) resolves `devcroft.toml` from
  `cwd` and accepts `[name]` or `--name <n>`, **not** `--sandbox <name>`.
  Remediation commands must use valid `up` syntax rooted at `project_root`.
- Sweeping unreferenced `agents/` objects on `rm` deletes an object installed
  by `devcroft agent install` before its first `up` unless explicit installs
  hold a retention pin.
- `procfs` is a Linux Bun requirement (`/proc/self/maps`); macOS has no `/proc`
  at all, so `procfs = true` must be platform-scoped.
- `exec::shell()` enables raw mode after `SpawnOk` and writes no session output
  before that, so a query cannot reach the terminal, and its reply cannot
  arrive, while the local TTY is cooked. What is wrong is narrower: `tcsetattr`
  results and `EINTR` are ignored, and the forwarded signals are blocked only
  inside `stream_session`, after raw mode, so a signal in between skips the
  restore. Moving raw mode before `Spawn` would fix nothing and would disable
  `ISIG` while the client waits on the keeper, so Ctrl-C could not end a hung
  connect.
- Linking the `jdx/mise` crate at runtime was evaluated and rejected: `mise` is
  not a public SemVer API (`doc = false`), caches `MISE_DATA_DIR`/`HOME` in
  process-wide `LazyLock` statics (requiring `unsafe` `std::env::set_var` under
  Rust 2024 or subprocess re-exec), runs downloaded `node`/`go`/`python`
  binaries host-side in its core installers, and pulls ~20 internal workspace
  crates into the binary that also serves as keeper, base runtime and microVM
  guest payload. Pure-Rust materialization paired with a dev-only recipe
  generator over a committed catalog and `aqua-registry` has zero runtime cost.

What exists and is reused:

| Piece | Where | Reused for |
|---|---|---|
| Content-addressed read-only store, leases, sweep, completion record | `src/provider/artifacts.rs` | agent objects and explicit install pins |
| Descriptor-anchored `O_NOFOLLOW` directory traversal and atomic write | `src/runtime/owned_dir.rs` | symlink-safe reads and merges in sandbox `HOME` |
| Header walk for host libraries | `artifacts::header_dependencies` | precondition check on native binary dependencies |
| Confined provisioning phase with its own network | `sandbox-provisioning` (`__provision`) | recipes whose install must run code |
| Credential broker and `[credentials] env` custody rules | `add-agent-workload` 4b | recipe `api-key` and `env` auth modes |
| Sandbox `HOME`, `<project>/.devcroft/<name>/home` | `src/lifecycle/up.rs` | the agent's config, history and login |
| Piped and PTY sessions, SSH PTY relay | `src/exec.rs`, `src/keeper/`, `src/ssh/server.rs` | `agent acp`, `agent run`, OSC 7501 transport |
| Denial hook | `src/agent_hook.rs`, `init --agent claude-code` | wired idempotently into sandbox `HOME` |

## Goals / Non-Goals

**Goals:**
- From a clean host (no `mise`, `npm`, `nix` or preinstalled agent) to an agent
  answering ACP in a GUI-launched editor or running interactively in a
  terminal, with one line of user configuration.
- Both API-key users and subscription-login users work without mutually
  conflicting checks or accidental leakage via `[env] forward`.
- The agent's runtime never changes what the project's commands resolve through
  `PATH`, and inherited project variables in `reserved_env` never hijack the
  agent chain.
- Transparent bidirectional **OSC 7501** support across `agent run`, `devcroft
  shell` and `devcroft ssh` PTY sessions, with clean JSON-RPC stdio on `agent
  acp`.
- Every agent artifact verified, pinned when explicitly installed, recorded by
  full recipe digest and reproducible from committed repository inputs alone.

**Non-Goals:**
- Separate policies, environments, `HOME`s or credential routes for agent and
  project (`add-agent-executor`).
- Containment of a hostile agent. One process tier, accident protection
  (`docs/threat-model.md`).
- Sharing one subscription login across sandboxes (`HOME` is per sandbox).
- Linking `mise` at runtime or reading gitignored working notes during builds.
- Importing the user's host agent configuration, MCP servers or history.
- A launcher for editors that run *inside* the sandbox (remote/SSH editors).
  `agent acp` and `agent run` are host verbs.
- Several conversations with separate lifecycles, resume, or sharing history
  between the CLI and ACP. What the harness does with its own `HOME` state is
  the harness's.
- A recipe update channel independent of devcroft releases. Between releases,
  the route is a user recipe (A1).
- A guided setup wizard.

## Decisions

### A1. Embedded recipes seeded from a committed catalog; user recipes

A recipe is TOML data, one file per agent, compiled into the binary from
`src/agent/recipes/*.toml`. Each embedded recipe carries one or more pinned
versions; `[agent] version` selects one, defaulting to the newest. A user
recipe at `~/.config/devcroft/agents/<name>.toml` adds a recipe or overrides an
embedded one under the exact same schema validator, with SHA-256/SHA-512
checksums mandatory on every artifact. `status`, `agent list` and
`policy --render` mark user recipes `(user recipe)`. Rollback is selecting the
previous version.

To keep embedded recipes reproducible from a clean `git clone` (never reading
gitignored `/new/`), the repository commits a versioned seed file
`scripts/agent-catalog.json` pinning upstream package coordinates and tags. The
research catalog it starts from carries no artifact digests and describes
itself as not an install manifest: per-platform URLs and checksums are resolved
and reviewed when a recipe is generated. It is paired with a dev-only generator (`scripts/` or a
non-shipped helper using `aqua-registry` metadata and upstream release
checksums) that writes `src/agent/recipes/*.toml`. The shipped `devcroft` crate
has zero dependency on `mise` or `aqua-registry`.

**Candidate and wave ordering (reconciled with `docs/agents.md`):**
- **Group 0 Candidate 1 (`single-binary`, `acp = "native"`, brokerable):**
  `opencode` (`goose` as backup), **for one named model provider**: the
  harness is multi-provider and a recipe has one broker, so the recipe pins the
  provider and says so. Neither is proven through devcroft's broker:
  `docs/agents.md` measured OpenCode against `nono-proxy` alone and Goose not
  at all, so task 0.7 decides. `cursor` is intentionally *not* Candidate 1:
  `docs/agents.md` measured that Cursor ignores `NO_PROXY`, opens `CONNECT` to
  loopback and trades its key for a session token, so it cannot be brokered.
- **Group 0 Candidate 2 (`acp = "adapter"`, npm + standalone CLI, broker +
  login):** `claude-code` (`aqua:anthropics/claude-code` for `interactive` and
  `@agentclientprotocol/claude-agent-acp` for `acp`).
- **Wave 1b / Wave 2 follow-ons:** `codex`, `github-copilot-cli`, `goose`,
  `cursor` (under `auth = "env"` / `"login"`, no broker), and `crush`
  (`acp = "none"`, CLI-only `agent run` + OSC 7501). `antigravity` requires
  `network.loopback = "any"` in the project manifest on Linux (A13); `pypi:`
  harnesses (`aider`, `mistral-vibe`) are out of scope until a Python wheel
  layout or confined installer is separately specified.

### A2. Recipe contents

```toml
name = "claude-code"                # matches `init --agent claude-code`
acp = "adapter"                     # "native" | "adapter" | "none"
default_auth = "api-key"            # used when no mode was chosen (A9)
state_paths = [".claude", ".claude.json"]   # relative to HOME; used by mode 1->2 cleanup

[[version]]
version = "2.1.331"                 # in the object id
[[version.artifact]]                # one per component per platform
role = "harness"                    # runtime | harness | adapter
component_version = "2.1.331"       # an adapter may bundle another harness build
platform = "linux-aarch64"          # linux-aarch64 | linux-x86_64 | macos-arm64 | macos-x86_64
url = "https://…"
sha256 = "…"
layout = "single-binary"            # single-binary | archive | node-tarball | npm-lock

[launch]
acp = ["{runtime}/bin/node", "{adapter}/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js"]
interactive = ["{harness}/claude"]
command = "claude"                  # the launcher's name on PATH
# Variables stripped when starting the agent or launcher so an inherited
# project value cannot override the pinned binary or trigger argv secret leaks.
reserved_env = ["CLAUDE_CODE_EXECUTABLE"]

[launch.env]                        # harness-specific names only (A4)
DISABLE_AUTOUPDATER = "1"           # illustrative; Group 0 measures real levers

[network]
hosts = ["api.anthropic.com"]

[auth.api_key_broker]               # optional; omitted for non-brokerable agents (e.g. cursor)
env = "ANTHROPIC_API_KEY"
host = "api.anthropic.com"
header = "x-api-key"
endpoint_env = "ANTHROPIC_BASE_URL"
placeholder = "sk-ant-api03-{}"

[auth.login]                        # optional; qualified per recipe in Group 0
hosts = ["console.anthropic.com", "claude.ai"]

[auth.env]                          # optional; only for non-brokerable keys (e.g. CURSOR_API_KEY)
env = "CURSOR_API_KEY"

[requires]
commands = ["git", "rg"]            # resolved on the sandbox's PATH at `up` (warn if absent)
runtime_env = ["NODE_OPTIONS", "NODE_PATH"]

[requires.linux-aarch64]            # platform-scoped (A13): never applied to macos-*
procfs = true
[requires.linux-x86_64]
procfs = true

[hook]                              # A10, optional
denial = "claude-code"
```

Placeholders (`{runtime}`, `{harness}`, `{adapter}`) resolve only inside the
recipe version's immutable object. `state_paths` lists relative single-level or
subdirectory names inside `HOME` that belong to this harness (validated free of
`..` and leading `/`), consumed by `add-agent-executor` when cleaning shared
mode 1 state on transition to mode 2.

### A3. Pure-Rust materialization executes nothing downloaded

devcroft materializes recipe objects in Rust over `src/provider/artifacts.rs`
(`hyper`, `rustls`, `flate2` and `sha2` are already in `Cargo.lock`; `tar` and
any needed decompressor are added with `THIRD-PARTY-LICENSES.md` regenerated):
- `single-binary`, `archive`, `node-tarball`: verify SHA-256 against streamed
  bytes before publishing, then extract. Archive extraction refuses absolute
  paths, `..` components, device/special entries, and symlinks or hardlinks
  whose targets escape the staging object.
- `npm-lock`: the recipe embeds or pins a lockfile v3 (`package-lock.json`).
  devcroft lays out `node_modules/` directly in Rust: verifies each tarball's
  SRI `integrity` (`sha512`), strips the leading `package/`, extracts under the
  same path/link sanitizer, selects platform-specific optional packages
  (including `@anthropic-ai/claude-agent-sdk`'s native platform payload) by
  `os`, `cpu` and `libc` (glibc vs musl), and creates `node_modules/.bin`
  links. It runs **no** lifecycle scripts. Any selected package with
  `hasInstallScript: true` refuses the recipe unless marked unnecessary from
  Group 0 evidence.
- Recipes needing code execution at install run only inside confined
  provisioning (`__provision`), writing solely into the staging object, and
  `status` marks them.

### A4. Same sandbox, private runtime, `reserved_env` stripped

The agent starts through the keeper by absolute `argv` into its object, and
nothing from its runtime is placed on `PATH`:
- Tools the agent spawns through `PATH` resolve to the project's environment
  (Node 16 project, Node 24 agent), while `child_process.fork()` stays on
  `process.execPath`.
- **`reserved_env` protection:** `SpawnRequest` gains `unset` (serde-defaulted),
  applied last, after the activation overlay and the request's `env`, so neither
  the provider nor an activation script can put a name back. `agent acp` and
  `agent run` send the recipe's `reserved_env` there; the launcher (A5) unsets
  them itself. They are also refused at layer `config` in `manifest.env.vars`
  and `manifest.env.forward`. Because `SpawnRequest` does not reject unknown JSON
  fields on the wire (`src/keeper/protocol.rs`), `agent acp` and `agent run`
  verify support from `Meta` before connecting: a keeper started before this
  feature has no recorded agent recipe in `Meta` (and `Meta` records a keeper
  protocol version for future extensions), so an older keeper is refused before
  `Spawn`, naming `up [--name <n>] --recreate`.
- **`[launch.env]` is per launch:** set on the agent's processes by the spawn or
  the launcher, never in the keeper's environment, so project commands that the
  agent did not start do not see it.
- **Project → agent:** if the project's resolved environment sets any name in
  `requires.runtime_env` (`NODE_OPTIONS`, `NODE_PATH`, `LD_*`, `DYLD_*`), `up`
  prints one warning naming the variable and recipe.
- **Agent → project:** `[launch.env]` rejects `PATH`, `HOME`, `XDG_*`, `LD_*`,
  `DYLD_*`, `requires.runtime_env` and `reserved_env` names at schema
  validation.

### A5. Launcher in the sandbox's artifact directory, after the project on PATH

A launcher must unset `reserved_env` and set `[launch.env]`, so it cannot be a
symlink to a native binary, and it must not be a shell script. It is therefore
the same file for every harness: `<project>/.devcroft/<name>/agent/bin/<command>`,
whose shebang is `#!<devcroft executable> __agent-launch` and whose body is the
resolved launch record (absolute `argv`, variables to set, variables to unset).
`__agent-launch` reads that file, adjusts the environment and execs.

It is per sandbox and rewritten on every `up` through `OwnedDir`, because it
names the running devcroft's path and the resolved recipe; inside an immutable
shared object it would go stale on upgrade. Before writing it (in A7a), `up`
verifies that the canonical `devcroft` executable path contains no ASCII
whitespace or newlines (which kernel shebang parsing cannot escape) and that
`#!<devcroft executable> __agent-launch\n` fits within the platform's kernel
shebang limit (`BINPRM_BUF_SIZE` on Linux, `SHSIZE` on macOS), failing at layer
`backend` naming the path if not. `with_read_only_artifacts` denies writes to
the directory from inside, the `devcroft` executable is already granted
(`with_keeper_exe_grant`), and `<project>/.devcroft/<name>/agent/bin` receives
its own read-only grant with origin `agent:<recipe>` (A11) so manifests that do
not grant `.` still mount and execute the launcher. The launch record holds no
secret. `PATH` order is: project entries → `[tools]` layer → launcher directory
→ base runtime fallback. Shadowing is reported by `up` and `status`.

### A6. State in HOME, with `O_NOFOLLOW` descriptor-anchored merges

The agent's config, history, cache and subscription login live in the sandbox's
own `HOME` (`<project>/.devcroft/<name>/home`), never under
`~/.local/share/devcroft` and never inside the read-only object.

Because `HOME` sits inside a directory the sandbox can write, **every host-side
read, merge or write inside `HOME` MUST extend `src/runtime/owned_dir.rs`'s
descriptor-anchored discipline**:
- Every directory component from the project root through `.devcroft/<name>/home`
  and any managed subdirectory (e.g. `.claude`) is created/opened step by step
  with `mkdirat` + `openat(..., O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`.
  Any symlinked directory component fails `up` at layer `config` naming the
  path, leaving the symlink target untouched.
- Existing managed config files are read via `openat(..., O_RDONLY | O_NOFOLLOW | O_CLOEXEC)`
  checking `fstat` for a regular file within a bounded size limit; a symlink or
  special file at the target path is refused at layer `config` (not followed
  and not silently clobbered if the user planted a dotfile link), and malformed
  JSON/TOML fails at layer `config` naming the file.
- Updates merge only devcroft's owned keys (preserving user preferences and
  auth tokens across `down`, `up` and `up --recreate`) and commit via exclusive
  temp file (`O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW`) + `renameat` in the
  anchored directory descriptor. Only `devcroft rm` deletes `HOME`.

### A7. Host-side `agent` verbs and deterministic selection precedence

`devcroft agent` has two disjoint verb sets:
- **Host-side:** `list`, `install`, `uninstall`, `acp`, `run`. Never require
  `own_sandbox_dir`.
- **In-sandbox:** `attention`, `progress`, `done`, `failed`, `hook`.

**One selection rule** (owner's decisions, 2026-11-24: `--recreate` keeps the
selection; a sandbox that never had a choice adopts `[agent]` only at
`--recreate`, never at plain `up`; switching recipes inherits neither `version`
nor `auth` from the old recipe):
1. `--agent <recipe>[@<version>]`, `--no-agent` and `--agent-from-config` are
   mutually exclusive; passing `--agent-auth` with `--no-agent` (or when the
   effective selection has no agent) fails at layer `config`.
2. **New sandbox (`Meta` does not exist on disk yet):** an explicit flag wins,
   else `[agent]` from `~/.config/devcroft/config.toml` (or `Disabled` / `Unset`
   when `[agent]` is absent), and the resulting selection is recorded in `Meta`.
3. **Existing sandbox (`Meta` exists on disk, whether running or `down`):**
   - Without `--recreate`, the effective selection MUST equal `Meta`'s recorded
     state (`Unset` stays `Unset` with no agent; `Disabled` stays `Disabled`;
     `Selected` keeps its recipe, version and auth mode). Passing a flag that
     would change `Meta`'s recorded state (including adding an agent to an
     `Unset` or `Disabled` sandbox while `down`) fails at layer `config` naming
     the same command with `--recreate`, so egress never widens without an
     explicit recreate.
   - With `--recreate`: an explicit flag (`--agent`, `--agent-auth`,
     `--no-agent`, `--agent-from-config`) wins and updates `Meta`; otherwise a
     recorded `Selected` or `Disabled` choice in `Meta` stands unchanged, while
     a sandbox whose `Meta` has `Unset` (never had a selection) reads and
     records `[agent]` from `~/.config/devcroft/config.toml`!

Changing `[agent]` in the user configuration therefore never changes a recorded
selection (`Selected` or `Disabled`), and never widens an `Unset` sandbox until
`--recreate`. `status` says when the user configuration's default differs, and
`up --agent-from-config --recreate` adopts it.

**Recipe switching (`R_new != R_old`).** Recorded `version` and `auth_mode`
belong strictly to the recipe they were recorded for, and `[agent]`'s `version`
and `auth` belong strictly to the recipe `[agent]` names:
- When keeping the same recipe `R` (without `--agent-from-config`): `@<version>`
  and `--agent-auth` override if passed; otherwise `Meta`'s recorded requested
  `version` and `auth_mode` for `R` stand.
- When switching to a different recipe `R_new` (or for a new/`Unset` sandbox, or
  under `--agent-from-config`): nothing is inherited from `R_old`. Requested
  `version` is `@<version>` if passed, else `[agent] version` when `[agent]`
  names `R_new`, else newest; `auth_mode` follows A9 for `R_new`.

**Version and recipe drift.** `Meta` records the resolved recipe itself (and
`mode1_recipes`: the set of recipes that have ever run in mode 1 on this
sandbox, consumed by `add-agent-executor` E6). Plain `up` after `down` runs that
recorded recipe, re-materializing from the record if the object is gone, even
when the installed devcroft no longer ships the version. `--recreate`
re-resolves: the requested version if there was one, else the newest. If a
requested version is no longer available, it fails naming the available ones,
before any teardown.

**Running sandbox.** If the sandbox is up and `--recreate` was not passed, `up`
reports `already up` and prints the recorded agent and its hosts when the
effective selection equals `Meta`'s, or fails at layer `config` naming
`--recreate` when a flag asks for a different selection.

### A7a. Everything checkable is checked before teardown

`up` reads health without acting on it, and then, before it terminates a
keeper, resolves the provider or creates state, it:
1. resolves the selection and validates the recipe and platform;
2. runs the auth, broker, `[credentials]` and collision checks (A9);
3. checks host-side requirements (`procfs`, `loopback_any`);
4. takes the `agents` lease and materializes the object.

The existing `[credentials]` and `check_brokers` checks move with it. They run
today after provider resolution, and under `--recreate` after the keeper is
gone; that is a defect for the user brokers already shipped, so the move is its
own task and lands first (tasks 1a). A sandbox that is already up and not
being recreated runs none of these checks, so a key missing from a later
shell does not fail it. Only checks that need the resolved environment
(`requires.commands`, `runtime_env`) run after the provider resolves, and they
warn rather than fail.

### A8. Nested-safe discovery, bounded `agent acp` readiness, and valid `up` remedies

`devcroft agent acp [<recipe>] [--sandbox <name>] [--print-config [<editor>]]`
and `devcroft agent run [<recipe>] [--sandbox <name>] [-- args…]` resolve their
target `(project_root, sandbox_name)` without ever letting an outer parent
sandbox swallow a nested subproject:

1. **Explicit `--sandbox <name>`:** looks up `<name>`'s recorded `Meta` (if any)
   or matches `<name>` against the manifest discovered from `cwd`.
2. **Innermost-root comparison (when `--sandbox` is omitted):** devcroft finds
   *both* (a) the nearest ancestor directory containing `devcroft.toml`
   ($R_{\text{manifest}}$) and (b) the innermost recorded `Meta` whose canonical
   `project_root` contains canonical `cwd` ($R_{\text{meta}}$), and picks
   whichever root is **deeper (more specific)**:
   - If $R_{\text{manifest}}$ is deeper than $R_{\text{meta}}$ (a nested
     checkout or subproject with its own `devcroft.toml` that has never run
     `up`), $R_{\text{manifest}}$ wins — it never attaches to the outer parent
     sandbox.
   - If $R_{\text{meta}} = R_{\text{manifest}}$ (or only $R_{\text{meta}}$
     exists), recorded `Meta` entries at that root are used; if two sandboxes
     are recorded for that same `project_root` (e.g. via `up --name`), fail at
     layer `config` listing them and showing `--sandbox <name>`.
   - If neither exists (e.g. GUI editor started with `cwd = /` or `$HOME`), fail
     immediately naming `--sandbox <name>`.

**Valid remediation command formatting (matching `src/bin/devcroft.rs:2758,4861`):**
`devcroft up` takes `[name]` / `--name <name>` resolved from `cwd` — it has no
`--sandbox` flag. Every remediation command printed by `agent acp`, `agent run`
or `status` is formatted from `(project_root, sandbox_name, manifest.sandbox.name)`
so it executes verbatim:
- Every remedy keeps the recorded selection (A7), so none can silently change
  the agent, its auth mode or an opt-out.
- Default sandbox name (`sandbox_name == manifest.sandbox.name`):
  `devcroft up --recreate` (plus `--agent <recipe>` when switching), prefixed
  with `cd <quoted project_root> && ` whenever caller `cwd` is outside
  `project_root` (or inside a nested subproject).
- Overridden sandbox name (`sandbox_name != manifest.sandbox.name`):
  `devcroft up --name <sandbox_name> --recreate` (with the same `cd` prefix
  when needed).

**Preflight, bounded auto-up, and GUI editor onboarding:**
- `agent acp --print-config [<editor>]` prints the ready-to-paste ACP launch
  configuration (using the resolved absolute `devcroft` executable path so GUI
  editors with a minimal `/usr/bin:/bin` `PATH` find `devcroft` without shell
  profile sourcing).
- On normal `agent acp` invocation:
  - Refuses `acp = "none"` at layer `config`, naming `devcroft agent run <recipe>`.
  - Runs synchronous preflight before any `up`:
    - If `Meta` already exists on disk (whether the sandbox is running or
      `down`) and its recorded selection is `Unset`, `Disabled`, or a recipe
      other than the target one, fails immediately on `stderr` naming the valid
      `devcroft up [--name <n>] --recreate [--agent <recipe>]` command — never
      starting an agent-less `up` on an `Unset` sandbox and never adding an
      agent without `--recreate` (A7).
    - Verifies the recipe's object is installed (never downloading in `agent
      acp`), auth prerequisites are configured when the sandbox is down
      (checking that the host variable is non-empty or `[agent.credentials.<recipe>]
      command` is declared, **without executing `command` in preflight** so
      Keychain/TouchID/`op` prompts run only once inside `up`), and required
      project lockfiles exist. Any failure exits immediately on `stderr` with
      the exact remediation command.
  - When the sandbox is down (either a brand-new sandbox with no `Meta` yet, or
    a stopped sandbox whose `Meta` already records this recipe), starts
    `devcroft up` as a **separate process** in its own session (`setsid`), with
    `stdin` from `/dev/null` and both outputs to a log beside the lifecycle
    lock, passing `--agent <recipe>` only when no `Meta` exists yet and
    `<recipe>` was named on the command line, and waits for it under a bounded
    `ACP_AUTO_UP_DEADLINE` (calibrated in task 0.9; default 10s). That `up` owns
    its own completion: at the deadline, or if the editor kills `agent acp`, it
    is left running, and `agent acp` exits non-zero on `stderr` saying the
    sandbox is still being prepared, naming the log and
    `cd <project_root> && devcroft up [--name <n>]`. Killing it instead would
    make every editor retry start the same provisioning again.
  - When the lifecycle lock is already held, waits under the same deadline and
    starts no second `up`. `down` and `rm` serialize on that lock as today. If
    an earlier detached `up` exited non-zero, the next `agent acp` prints a
    concise summary of that earlier error to `stderr`, clears the failure
    marker, and starts a fresh detached `up` under the deadline so transient
    failures self-heal once fixed.
  - Staleness alone does not refuse: a running sandbox whose recorded recipe
    differs from the installed one is used, with one `stderr` line naming the
    recreate command. Requesting a *different recipe* than the recorded one
    fails with the formatted `up [--name <n>] --recreate --agent <recipe>`.
    Nothing is ever recreated implicitly.
  - Passes `stdin` and `stdout` directly (`pty: None`); zero devcroft messages
    or OSC sequences ever touch `stdout`.
- **ACP lifecycle.** Client `stdin` EOF is forwarded and the agent gets a grace
  period to exit; after it, or when the client process dies, the keeper ends
  that session's process group and nothing else. The sandbox, its services and
  other sessions stay up. Cancelling one ACP request is a JSON-RPC message that
  devcroft passes through and does not interpret.
- Interactive `agent run` checks the same `Meta` rule first (failing with the
  `--recreate` remedy if `Meta` exists with `Unset`, `Disabled` or another
  recipe), materializes a missing recipe object automatically with progress on
  `stderr`, and waits normally for `up` on a new or matching stopped sandbox.

### A9. Credentials: one mode per sandbox, one source per variable

A recipe declares supported modes (`[auth.api_key_broker]`, `[auth.login]`,
`[auth.env]`, optionally scoped per platform when macOS `HTTPS_PROXY` userinfo
behaviour differs in Group 0.7) and `default_auth`, which must be one of them.

**Which mode for target recipe `R`:** an explicit `--agent-auth`, else `Meta`'s
recorded `auth_mode` **only when `Meta` already records recipe `R` and
`--agent-from-config` was not passed** (never inherited from a different old
recipe, A7), else `[agent] auth` when `[agent]` in the user configuration names
`R`, else `R`'s `default_auth`. The shell's contents never choose the mode. If
the chosen mode's prerequisite is missing (or unsupported on this platform),
`up` fails at layer `config` naming it and every other mode the recipe supports
on this platform.

**Where a key comes from** (`api-key` and `env`): the host variable when it is
non-empty, else the user configuration's source for that recipe:

```toml
[agent.credentials.claude-code]
command = ["security", "find-generic-password", "-w", "-s", "anthropic"]
```

`command` is an `argv`, run host-side without a shell, with a deadline, once per
keeper start. It is read from the user configuration only; a project cannot
declare one. Its output is trimmed of one trailing newline, is never logged or
recorded, and goes where the variable's value would have gone: to the broker
under `api-key`, into the sandbox under `env`. This is what makes auto-up work
from an editor started by the OS, whose environment holds no key. When the
sandbox is already up, `agent acp` needs no key.

**Collisions**, all at layer `config` before teardown (A7a):
- In **every** mode, the recipe's credential names (`env` and `endpoint_env` of
  the broker, `env` of `[auth.env]`) may not be named by `manifest.env.vars`,
  `manifest.env.forward` or `[credentials] env`. Under `login` this is what
  stops a forwarded API key from replacing the subscription; under `api-key` the
  placeholder would win anyway, and the refusal removes the ambiguity.
- In every mode, a user `[[credentials.broker]]` for one of the recipe's active
  hosts is refused, since a brokered host is reachable only through its
  endpoint. The exception is an entry identical to the recipe's broker under
  `api-key` (same host, header, `env` and `endpoint_env`): it is used once and
  `up` says so. A shared `env` or `endpoint_env` with another host is refused.
- `api-key` with `network.default = "allow"` is refused naming `--no-agent`.

`login` registers no broker, adds `[auth.login].hosts` with origin
`agent:<recipe>`, and announces that the token in `HOME` is readable by the
project. `env` is for non-brokerable recipes only and prints the
delivered-credential custody disclosure.

### A10. Denial feedback without committed files

For recipes declaring `[hook] denial = "<agent>"`, `up` merges the user-level
denial hook into the sandbox's `HOME` through A6's `O_NOFOLLOW` descriptor-anchored
merge. Nothing is committed to the project.

### A11. Policy composition and the agent's origin

Agent rules carry origin `agent:<recipe>`: read-only access to `<object>`,
read-only access to `<project>/.devcroft/<name>/agent/bin` (the per-sandbox
launcher directory, independent of whether the manifest grants `.`),
`[network].hosts`, `[auth.login].hosts` (under `auth = "login"`), and
`[launch.env]`. `up` composes manifest + provider + tooling + agent rules
before `check_brokers`, `ensure_egress_proxy`, `policy --render` and
`CapabilityPlan` compilation, and prints the added egress hosts at `up`. Native
binaries in `<object>` pass `artifacts::header_dependencies` against the
profile's library inventory.

### A12. Lease, install pins, and staleness

- `up` holds the `agents` store lease across materialization and the `Meta`
  write for every project provider, then sweeps.
- `devcroft agent install <recipe> [--version <v>]` writes
  `agents/pins/<object-id>` (holding the recipe, version and source) under the
  store lock. Pins name the object and not `<recipe>-<version>`, because a user
  recipe may share both with an embedded one. `sweep` retains objects that a
  `Meta` or a pin names. `uninstall` removes every pin recorded for that recipe
  (and version), then sweeps.
- `Meta` records the selection, the resolved recipe, `source`, `auth_mode`,
  `object_id` and `recipe_digest` (SHA-256 over the canonical resolved recipe
  and `auth_mode`). A sandbox is stale when its recorded selection would now
  resolve to another digest: after a devcroft upgrade or an edited user recipe,
  never after a change to `[agent]`, which does not apply to it (A7).
- Objects keep the store's usual `0755`/`0644`. They are immutable to the
  sandbox because the policy grants them read-only, and because
  `artifact_store_write_overlap` refuses a write grant over the store.

### A13. Platform-scoped requirements

`up` merges `[requires]` with `[requires.<current-platform>]` before
restriction. `procfs = true` lives under `[requires.linux-*]` only, never
refusing macOS. Missing `commands` on `PATH` warn once by name. `loopback_any =
true` requires `network.loopback = "any"` in `devcroft.toml`.

### A14. Bidirectional OSC 7501 (`herdr-core`) terminal transparency

In **OSC 7501** (`herdr-core` / `hc ls`), the agent writes a capability query to
its TTY, the terminal emulator replies on `stdin` **without a newline**, and the
agent then emits identity and state reports. The exact framing comes from the
`herdr-core` revision that task 0.1 pins; this design fixes none of it.

The contract belongs to the transport, not to recipes: the requirements are in
this change's `exec` and `ssh` deltas and their tasks (5a) depend on no recipe,
so they can land first. `agent run` is one more client of it.

- **Terminal mode discipline in `src/exec.rs`** (one client for `shell` and
  `agent run`): block the forwarded signals, then enable raw mode, both after
  `SpawnOk` and before any session byte is read or written. Retry `tcsetattr` on
  `EINTR`; failing to enter raw mode on a TTY ends the session with an error
  rather than running cooked. Restore on every exit, including a forwarded
  signal. Raw mode stays off until `SpawnOk`, so Ctrl-C ends a client waiting on
  the keeper.
- **Byte-exact bidirectional relay:** `keeper::connection`, `exec::stream_session`
  and `ssh::server` forward arbitrary chunks in order without UTF-8
  re-encoding, OSC parsing, buffering until newline/terminator, or filtering
  unknown extensions (`T01`–`T05`, `T09`, `T12`).
- **No emulation, no fabricated state, strict session isolation:** devcroft
  never answers a capability query on behalf of a silent terminal (`T06`), never
  invents agent identity or states (`done`, `idle`, `blocked`) for non-speaking
  programs (`T07`), never broadcasts terminal replies across concurrent or
  reconnected sessions (`T08`, `T10`), grants zero sandbox authority from OSC
  payloads, and emits zero OSC sequences on `agent acp` `stdout` (`T11`).
- **Log sanitization (`OSC-09`):** session PTY streams are never written to
  `paths.log`, and `devcroft logs` (`src/bin/devcroft.rs:3244`) strips ESC/OSC
  control sequences whenever `stdout` is a TTY so hook or provisioning output
  recorded in `paths.log` cannot replay historical OSC 7501 sequences as live
  events.
- **Test fixture discipline:** `openpty` (`src/keeper/pty.rs`) starts `slave_fd`
  in cooked mode, matching real login PTYs; synthetic conformance probes in
  `tests/osc7501_pty.rs` switch their own `STDIN_FILENO` to raw mode before
  emitting the query, matching real TUI agents.

## Risks / Trade-offs

- [Agent shares project policy, environment and `HOME` in mode 1] → stated in
  `status` and docs; `reserved_env` and the collision
  checks prevent accidental overrides; `add-agent-executor` isolates trees.
- [Symlinks planted in writable `HOME`] → mitigated by `O_NOFOLLOW`
  descriptor-anchored traversal and `O_EXCL` + `renameat` (`OwnedDir`, A6).
- [Cold project provisioning behind GUI editor `agent acp`] → the background
  `up` finishes on its own after the deadline, so a retry succeeds (A8).
- [A recipe may bundle two builds of one harness, as `claude-code` does: a
  standalone CLI and the one inside the adapter's SDK, sharing one `HOME`] →
  each component's version is recorded and shown, and task 0.2 tests both
  orders of use on one `HOME`.
- [`[agent.credentials]` runs a user-chosen host command] → user configuration
  only, an `argv` without a shell, and its output never recorded.
- [Separate sandbox UID (`add-sandbox-identity`)] → artifact store objects use
  `0755`/`0644` modes, matching `mise` and `pixi`.

## Open Questions

- Should one sandbox allow multiple installed recipes simultaneously for
  switching between two editors/agents without `--recreate`?
- Should `devcroft.toml` gain an advisory recommended recipe name (never
  overriding user `[agent]` or `--no-agent`)?

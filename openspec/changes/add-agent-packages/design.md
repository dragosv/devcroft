## Context

Requirement (owner, 2026-11-19, `new/agent-runtime-workspace-separation-2026-11-19.md`):
an agent with its ACP adapter must be easy to add and must not depend on the
project's provider. The adversarial review of that note
(`new/adversarial-review-agent-runtime-workspace-separation-2026-11-19.md`)
recommends two process trees and an RPC executor. That is the strong form,
and it is split into `add-agent-executor`. This change is the form that
delivers the requirement inside today's keeper model, where one sandbox runs
under one policy. A second review (2026-11-22) checked this design against
the code; its findings are folded in below, each where it changed a decision.

What exists and is reused:

| Piece | Where | Reused for |
|---|---|---|
| Content-addressed read-only store, leases, sweep, completion record | `src/provider/artifacts.rs` | agent objects |
| Header walk for host libraries | `artifacts::header_dependencies` | precondition check on a native binary's dependencies |
| Confined provisioning phase with its own network | `sandbox-provisioning` (`__provision`) | recipes whose install must run code |
| Credential broker | `add-agent-workload` 4b | the recipe's broker template |
| Sandbox `HOME`, `<project>/.devcroft/<name>/home` | `src/lifecycle/up.rs` | the agent's config, history and login |
| Piped and PTY sessions | `devcroft exec`, `devcroft shell` | `agent acp`, `agent run` |
| Denial hook | `src/agent_hook.rs`, `init --agent claude-code` | wired by the recipe instead of committed files |

Facts from the code that constrain this design:
- `<state>/<name>/` is under `~/.local/share/devcroft`, which is a baseline
  deny; `why_path` lets a deny win over any allow. Nothing of the agent's may
  live there.
- `check_brokers` and the egress-proxy decision compile the manifest alone,
  and `maybe_auto_up` starts with `UpOptions::default()`, ignoring the user
  configuration.
- On macOS, exec is scoped to read grants; `/bin/sh` is refused at `execve`.
- `cli_agent` exits 2 outside a sandbox: today `devcroft agent` is the
  in-sandbox namespace.
- The artifact lease in `up` is taken only when the *project's* provider is a
  store provider.
- `header_dependencies` refuses universal (fat) Mach-O and returns sonames,
  not host paths.
- `devcroft exec` never allocates a PTY.

## Goals / Non-Goals

**Goals:**
- From a clean host to an agent answering ACP in a sandbox, with one line of
  user configuration, one exported API key and no agent preinstalled.
- The agent's runtime never changes what the project's commands resolve
  through `PATH`.
- Every agent artifact verified, recorded and visible in `status` and
  `policy --render`.
- Agent versions move without a devcroft release, and roll back.

**Non-Goals:**
- Separate policies, environments or credential routes for agent and project
  (`add-agent-executor`).
- Containment of a hostile agent. One process tier, accident protection
  (`docs/threat-model.md`).
- Sharing one login across sandboxes. A subscription login lives in one
  sandbox's `HOME` (A9).
- Several agents in one sandbox (an open question).
- A second VM.

## Decisions

### A1. Recipes ship with devcroft; users may add their own

A recipe is TOML data, one file per agent, compiled into the binary from
`src/agent/recipes/`. Each embedded recipe carries several versions;
`[agent] version` pins one, and the default is the newest. A user recipe at
`~/.config/devcroft/agents/<name>.toml` adds a recipe or a version, under the
same schema validator, with a checksum mandatory on every artifact. A user
recipe that shares a name with an embedded one takes precedence, and
`status` and `agent list` mark it `(user recipe)`. Rollback is selecting the
previous version; its object stays in the store until swept.

*Alternatives considered:*
- Embedded only. Rejected after review: vendors ship several times a week,
  and a user who must wait for a devcroft release will bypass devcroft.
- A remote catalog, fetched at `up`. Rejected for the first slice: another
  resolver at provisioning time, for data that names entrypoints and hosts.
- The ACP registry alone. Rejected as the source of truth: it provides
  distribution metadata, not proof that a package is complete and starts
  without further downloads. It may seed recipes.

### A2. Recipe contents

```toml
name = "claude-code"                # matches `init --agent claude-code`
acp = "adapter"                     # "native" | "adapter"

[[version]]
version = "…"                       # in the object id
[[version.artifact]]                # one per component per platform
role = "harness"                    # runtime | harness | adapter
platform = "linux-aarch64"
url = "https://…"
sha256 = "…"
layout = "single-binary"            # single-binary | archive | node-tarball | npm-lock

[launch]
acp = ["{runtime}/bin/node", "{adapter}/dist/index.js"]
interactive = ["{harness}/claude"]
command = "claude"                  # the launcher's name on PATH

[launch.env]                        # harness-specific names only (A4)
CLAUDE_CODE_EXECUTABLE = "{harness}/claude"   # illustrative; gate measures the real lever
DISABLE_AUTOUPDATER = "1"                     # illustrative

[network]
hosts = ["api.anthropic.com"]

[auth.api_key_broker]               # A9
env = "ANTHROPIC_API_KEY"
host = "api.anthropic.com"
header = "x-api-key"
endpoint_env = "ANTHROPIC_BASE_URL"
placeholder = "…"

[requires]
procfs = true                       # a Bun binary aborts without /proc
commands = ["git", "rg"]            # resolved on the sandbox's PATH at `up`
runtime_env = ["NODE_OPTIONS", "NODE_PATH"]   # A4

[hook]                              # A10, optional
denial = "claude-code"
```

Names and values marked illustrative are placeholders; group 0 measures each
harness's real levers. Placeholders resolve only to paths inside the object.

### A3. Install executes nothing downloaded, by layout kind

- `single-binary`, `archive`, `node-tarball`: verify, then extract. Nothing
  runs. Archive extraction refuses absolute paths, `..`, and symlinks or
  hardlinks that resolve outside the staging object.
- `npm-lock`: the recipe pins a `package-lock.json` (lockfile v3). devcroft
  lays out `node_modules/` itself. For each entry it verifies the tarball
  against the lock's SRI `integrity` (`sha512`, base64), strips the tarball's
  leading `package/`, extracts to the install path under the same sanitizing
  rules, selects platform-specific optionals by `os`, `cpu` and `libc`
  (glibc vs musl), and creates `node_modules/.bin` links with their mode. It
  runs no lifecycle script. An entry with `hasInstallScript: true` refuses
  the recipe unless the recipe states, with measured evidence, that the script
  is not needed.
- A recipe that needs code at install runs it in confined provisioning
  (`__provision`), writing only into the staging object, with network limited
  to the recipe's install hosts. `status` marks it.

`npm-lock` is a small package manager written in Rust, and group 0 decides
whether it is worth it. The first recipe is a native-ACP single binary; the
npm adapter is the second, and may be dropped from the first slice (tasks
0.6).

*Alternative considered:* running a pinned `npm ci --ignore-scripts`
host-side. Rejected because it executes npm, which is downloaded code. The
same rule refused mise's core backends.

### A4. Same sandbox, private runtime, one environment

The agent process starts through the keeper like any session, by absolute
argv into its object. Nothing from the object is prepended to `PATH`.
Consequences:
- The agent's Bash-type tool spawns through the inherited `PATH`, so it gets
  the project's `node`, `npm` and libraries. This is the Node 16 / Node 24
  property for every command that resolves through `PATH`.
- `child_process.fork()` uses `process.execPath`, so the agent's own workers
  stay on its runtime. That is correct, not a leak.
- An adapter that starts the harness through `PATH` would pick the project's
  `node`. `[launch.env]` must pin every such executable. Group 0 detects
  misses with a sentinel `node` on the project's `PATH`.

The agent and the project share one environment, in both directions, and
this design states it rather than filtering:
- **Project → agent.** A project that sets a variable the agent's runtime
  reads (`NODE_OPTIONS`, `NODE_PATH`, `LD_LIBRARY_PATH`, `LD_PRELOAD`,
  `DYLD_*`) can break the agent. The recipe lists those names in
  `requires.runtime_env`. `up` prints one warning naming each one the
  project's environment sets, and the agent still starts. Unsetting them for
  the agent would unset them for every command it runs, which is the project
  breaking instead. `add-agent-executor` removes this.
- **Agent → project.** `[launch.env]` is part of the sandbox environment, so
  every command sees it. The schema therefore refuses `PATH`, `HOME`,
  `XDG_*`, `LD_*`, `DYLD_*` and the runtime names of `requires.runtime_env`
  in `[launch.env]`, and accepts harness-specific names only. Every value is
  visible in `policy --render` with origin `agent:<recipe>`.

### A5. Launcher inside the object, after the project on PATH

The launcher is generated into the object at install, as `<object>/launcher/<command>`,
and is covered by the object's digest. For a native binary it is a symlink to
the harness. For a script harness it is a file whose shebang is the object's
own runtime (`#!<object>/runtime/bin/node`) and whose body loads the harness
by absolute path. No launcher has a shell shebang: `/bin/sh` is refused on
macOS, and the sandbox's resolved shell differs per sandbox while the object
is shared. The directory goes on `PATH` after the project's entries and after
the tooling layer, and a shadowed launcher is reported the way
`tooling::shadowed` reports it. Only the agent's own command name is in that
directory. Group 0 measures the shebang-length limit against the store path
on both platforms.

### A6. State is the sandbox's HOME

The agent's config, history, cache and login live in the sandbox's own
`HOME`, `<project>/.devcroft/<name>/home`. It is already per sandbox, already
granted, already removed at `rm`, and it is never `~/.local/share/devcroft`.
No new write grant is needed, and no recipe moves `HOME` or `XDG_*` (A4).
Harnesses that write straight into `$HOME` (Claude Code's `~/.claude.json`)
land there. The project's commands share that `HOME`; that is the mode 1
property `add-agent-executor` changes.

### A7. Host-side `agent` verbs

`devcroft agent` has two verb sets. `list`, `install`, `acp` and `run` run on
the host and never require `own_sandbox_dir`. `attention`, `progress`,
`done`, `failed` and `hook` stay in-sandbox. A host verb run inside a sandbox,
or an in-sandbox verb run on the host, fails at layer `config` naming which
side it belongs to. `USAGE` lists both sets under their side.

### A8. ACP endpoint and interactive run

`devcroft agent acp [<recipe>] [--sandbox <name>]` is meant to be configured
once, globally, in an editor:
- **Which sandbox.** The one whose recorded project root contains the
  process's `cwd`, which also separates two checkouts of one project.
  `--sandbox` overrides. An editor that starts the command with `cwd = /` or
  `$HOME` and no `--sandbox` fails with that message.
- **cwd in the sandbox.** The `cwd` if it is inside the project root,
  otherwise the project root.
- **Auto-up.** If the sandbox is down, `agent acp` runs `up` with the user
  configuration loaded (`maybe_auto_up` changes to load it for every caller).
  It never downloads: if the recipe's object is absent it fails at once,
  naming `devcroft agent install <recipe>`, rather than letting the editor's
  `initialize` time out.
- **Wrong agent.** If the sandbox is up without that recipe, or with another
  version, it fails naming `devcroft up --recreate --agent <recipe>`. It
  never recreates by itself.
- **Streams.** stdin and stdout are passed through untouched; every devcroft
  message goes to stderr. devcroft does not parse ACP.

`devcroft agent run [<recipe>] [--sandbox <name>] [-- args…]` resolves the
same way and runs the recipe's `interactive` argv on a PTY, as `shell` does.

### A9. Credentials: the recipe owns the broker template

The recipe's `[auth.api_key_broker]` is a `[[credentials.broker]]` entry that
is active only in a sandbox that selects the recipe. The user writes
`[agent] name` and exports the key; nothing else. Policy composition (A11)
puts the recipe's hosts into the policy `check_brokers` reads, so a project
with `network.allow = []` passes. `check_brokers`' other rules stand:
- With `network.default = "allow"` the broker cannot guarantee its traffic
  passes the proxy, so `up` fails at layer `config` naming the agent, and
  suggests `--no-agent`.
- A user `[[credentials.broker]]` entry for the same host or the same `env`
  as the recipe's fails at `config`, naming both. Whichever is not meant is
  removed.

Subscription login (`auth.login`) is a second method, qualified per recipe in
group 0. The recipe names the login hosts, which join the allowlist with the
same origin. The token lands in the sandbox's `HOME`, survives `down` and
`up`, and is gone at `rm`. Every process in the sandbox can read it, and `up`
prints one line saying so, exactly as for a delivered `[credentials] env`
value. Whether the login flow works at all inside the sandbox (browser
redirect to a loopback port the host cannot reach, versus a paste-code flow)
is measured, not assumed.

### A10. Denial feedback without committed files

A recipe with `[hook] denial = "<agent>"` has `up` write the agent's
user-level hook configuration into the sandbox's `HOME`, pointing at the same
denial hook `init --agent` wires today. The project commits nothing. Group 0
measures that the harness reads a user-level hook from there, and that a
project-level setting does not silently override it.

### A11. Policy composition and the agent's origin

The agent contributes rules with origin `agent:<recipe>`: the object
(read-only; with the launcher, inside it), the recipe's hosts and login
hosts, and `[launch.env]`. Every check `up` makes reads the composed policy,
which is the manifest's policy plus the provider's grants plus the agent's
rules: `check_brokers`, the egress-proxy decision, the rendered and the
compiled profile. A manifest write grant overlapping the artifact store is
already refused at `config`.

On Linux, the header walk is a precondition check: a native binary's sonames
must be satisfied by the libraries the sandbox's profile already grants, as
for mise, and a miss refuses the recipe at layer `provider` naming the
library. Universal Mach-O is refused by the walk today; a recipe for macOS
pins a thin artifact, or group 0 qualifies selecting the host slice.

Selecting an agent widens the sandbox's egress by the recipe's hosts, from
the user configuration. `add-agent-workload`'s credentials spec forbids user
configuration from widening `network.allow`; this change amends that rule
for a selected recipe's own hosts, and `up` prints them on every run.

### A12. Lease, record, staleness

When an agent is selected, `up` takes the `agents` store lease for its whole
run, independently of the project's provider, and sweeps `agents` after.
`Meta` records the recipe, its version, its source (embedded or user) and the
object id. `status` prints `agent: <recipe> <version> (artifact tier)`, and
`agent: stale — run devcroft up --recreate` when the configured recipe or
version no longer matches `Meta`, as it does for `tools`.

### A13. Requirements the host must meet

`[requires]` is checked at `up`, before restriction. `procfs = true` refuses
where the sandbox has no procfs of its own (macOS today, or a Linux host
without a PID namespace), naming the recipe. `commands` are resolved on the
sandbox's `PATH`; a missing one is one warning naming it, since the agent may
still work without it. A recipe that would need `network.loopback = "any"`
is refused in this change, because that key is a project policy decision.

## Risks / Trade-offs

- [The agent shares the project's policy, environment and `HOME`] → stated
  in `status` and docs; `requires.runtime_env` warns about the known
  breakages; `add-agent-executor` is the remedy.
- [Harness chains look executables up on `PATH`] → A4's sentinel test is a
  qualification criterion per recipe; a miss blocks the recipe.
- [`npm-lock` is a package manager to maintain] → native binaries first;
  group 0 may drop npm adapters from the first slice.
- [User recipes are code-adjacent] → same validator, mandatory checksums,
  marked in `status`, `agent list` and `policy --render`.
- [Vendor self-update writes into the install] → the object is read-only.
  The recipe sets the vendor's update-disable switch, and a failed update is
  visible, not silent.
- [A subscription token is readable by the whole sandbox] → stated at every
  `up`, as for delivered credentials; the broker is the default method.
- [A separate sandbox UID cannot traverse the store] → the same constraint
  mise and pixi have; resolved in `add-sandbox-identity`, not here.

## Open Questions

- Should one sandbox carry several recipes, for an editor that offers two
  agents on one project?
- Should a project be able to declare `[agent]` for a team, beside the user's?
- Should a login be shareable between sandboxes of one user, and under what
  visibility?

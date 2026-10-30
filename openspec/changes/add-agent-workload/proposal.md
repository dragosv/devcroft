# Change: add-agent-workload

Status: proposed (post-MVP). Depends on: `add-mvp-core` complete.
Independent of `add-flox-services` and of the listening-socket gap — a
coding agent needs neither a service nor a listening port to edit code,
so this change is implementable today, unlike that one.

## Why

devcroft's stated audience is "fleets of coding agents" running in
parallel on one host. Verified live this session, that audience cannot
currently run at all: inside a real sandbox, the agent's runtime is
absent and its credentials are unreadable.

```
HOME=/home/vscode
ls ~/.claude  →  Permission denied
git           →  /usr/bin/git        ✓
node          →  NO_NODE             ✗
```

Every mainstream coding agent (Claude Code, Codex, OpenCode, Copilot CLI)
is a Node program authenticated either by an API key or by a
subscription login. Neither reaches into a sandbox today. The README
markets the fleet use case; the implementation does not support it.

The third gap is a correctness bug rather than a missing feature. Git
worktrees — the dominant local fan-out mechanism, and the one the README
identifies as both most-used and worst-served — each carry a copy of the
committed `devcroft.toml`, so every worktree declares the *same*
`sandbox.name`. State is keyed by name alone, so N worktrees silently
share one sandbox: one worktree's `up` serves another worktree's code,
with no warning. Verified by creating a real worktree.

## What Changes

- **Tooling layer, declared by the user, not the project** (decided
  2026-10-30). A user configuration file, `$XDG_CONFIG_HOME/devcroft/
  config.toml` (`~/.config/devcroft/config.toml` when unset), gains a
  `[tools]` section naming a second declarative environment, and `up
  --tools <path>` names one for a single invocation. It is resolved
  host-side at `up` and contributes **`PATH` entries only**, after the
  project's: never its env diff, so it cannot change the project's build.
  This is how an agent runtime gets inside without being declared as a
  project dependency, and without the team's repository saying anything
  about one developer's agent. It is a *declarative, locked*,
  closure-tier environment — **not** a host passthrough, which would be
  the `host` provider smuggled back in under another name.
- **Credential injection, split by auth shape** — because they are not
  the same problem — and requested in the same **user** configuration as
  the layer, never in the committed manifest:
  - API-key auth is delivered as an environment variable, with the
    exposure that implies stated. (The project-level `[env] forward`
    that already ships is unchanged: it names a variable, and holds no
    value.)
  - Subscription/OAuth auth is file-based (`~/.claude/.credentials.json`
    confirmed present), so env injection may not serve it. A new opt-in
    key grants the narrowest possible thing — a **single file,
    read-only** — never a directory, never silently. Whether read-only
    survives a token refresh, and whether an env-var form now exists,
    are both unmeasured (design.md Decision 2).
- **Sandbox identity is per project root, not per name.** `up` SHALL
  refuse to adopt a state directory recorded against a different project
  root, naming the conflict and the fix. `meta.json` already records
  `project_root`, so this is detection of an existing bug, not new
  bookkeeping. A `--name` override makes fan-out across worktrees
  explicit and scriptable.
- **BREAKING** (behavioral, narrow): a second worktree that today
  silently shares the first's sandbox will now fail `up` with an
  actionable error. This is the bug being fixed; anyone relying on the
  old behavior was relying on two checkouts sharing one environment
  without being told.

## Capabilities

### New Capabilities

- `tooling`: a second, user-declared environment whose `PATH` entries
  reach the sandbox for tools that are not project dependencies — where
  it is declared, which providers may serve it, its fixed position after
  the project's `PATH`, how it is recorded and goes stale, and the
  reproducibility bar it must meet.
- `credentials`: how a secret reaches a process inside the boundary —
  the two auth shapes, the narrowness requirement for file-based
  credentials, and what is disclosed to the user about the exposure.

### Modified Capabilities

- `config`: a user configuration file, new to devcroft, holding
  `[tools]` and the credential opt-in; validated with the same strictness
  as `devcroft.toml`, which itself gains neither.
- `lifecycle`: `up` refuses a state directory belonging to a different
  project root; `--name` selects an explicit sandbox identity; a tooling
  layer changed since `up` is reported stale.
- `cli`: `--name` on the commands that resolve a sandbox; `up --tools`;
  `up` discloses exactly which credential was exposed and how; `status`
  shows the layer and what it shadows; `doctor` reports whether a
  configured tooling layer resolves.

## Impact

- Affected specs: new `tooling`, `credentials`; modified `config`,
  `lifecycle`, `cli`.
- Affected code: `src/config/` (the user configuration file, new),
  `src/provider/` (resolving a second environment through the existing
  hook-free routes, keeping only its `PATH`), `src/policy/` (the
  layer's store grant and the credential file grant, each with its own
  origin so `policy --render` shows it), `src/lifecycle/up.rs`
  (project-root check, `--name`, `--tools`, the layer in `Meta`,
  credential handoff), `src/bin/devcroft.rs` (`--name`/`--tools`
  plumbing, disclosure output, `status`, `doctor`).
- `env-provider`'s "Fixed composition order" is about resolving *before*
  restriction, which the layer obeys unchanged; it says nothing about
  two environments. An earlier version of this proposal claimed to
  extend it. The ordering of project and layer is a new requirement, in
  `tooling`.
- The policy's inputs grow by one: manifest, provider grants, baseline,
  **and the tooling layer**, recorded in `Meta` so `policy --render`
  still renders from what `up` used, not from the user's configuration
  as it stands now.
- The "provider resolution must not widen the policy" invariant applies
  unchanged to the tooling layer: it may add a read-only store grant and
  nothing else.

## Success Criteria

- With an agent CLI in the user's tooling layer, an unmodified project
  comes up, and `devcroft exec -- <agent> --version` runs it inside the
  sandbox — without that agent, or any devcroft key naming it, appearing
  in any file the project commits. The project's environment, `PATH`
  aside, is byte-identical with and without the layer.
- The agent authenticates inside the sandbox under both auth shapes: an
  API key delivered as an env var, and a subscription credential
  delivered as a single read-only file.
- `policy --render` shows the credential grant with its own origin, and
  `up` prints exactly one line naming the file exposed. A credential is
  never granted implicitly by any other key.
- Two worktrees of the same repo produce two distinct sandboxes with
  `--name`, and produce a clear failure — not silent sharing — without
  it.
- A tooling layer cannot widen the policy: `policy --render` with one
  differs from without it only by a read-only store grant carrying a
  `tools:` origin.
- A tooling layer that is not reproducible (no lock, or a provider that
  cannot produce one, or one outside the closure tier) is rejected at
  `up`, not accepted with a warning.
- `up` and `status` say a layer is in use, and `status` names any binary
  of the layer's that the project's `PATH` shadows.

## Open Questions

Settled on 2026-10-30, kept so the reasoning is not re-litigated:

- ~~**Where the tooling layer is declared.**~~ The user's configuration,
  not the project. Design.md Decision 1.
- ~~**Whether the credential opt-in belongs in `devcroft.toml` at
  all.**~~ It does not; it follows the layer. Design.md Decision 2.
- ~~**What happens when the tooling layer and the project environment
  provide the same binary.**~~ The project's wins, and `status` shows
  the shadowed one. Design.md Decision 4.

Still open:

- **Turning a configured layer off for one invocation.** A user with a
  layer in their configuration has no way to bring up a sandbox without
  it, short of editing the file. A flag is the obvious answer; whether
  it is needed is better learned from use than decided now.
- **macOS parity, unmeasured.** Orca-style desktop fan-out is
  mac-heavy, and the tooling layer, credential grants, and per-root
  identity are all untested there. This is a verification gap, not a
  design unknown — but it could turn into one.

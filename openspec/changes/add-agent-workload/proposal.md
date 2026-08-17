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

- **Tooling layer.** `devcroft.toml` gains a `[tools]` section naming a
  second declarative environment, resolved host-side at `up` and composed
  into the sandbox alongside the project environment. This is how an
  agent runtime gets inside without being declared as a project
  dependency. It is a *declarative, locked* environment held to the same
  six-criterion bar as any provider — **not** a host passthrough, which
  would be the `host` provider smuggled back in under another name.
- **Credential injection, split by auth shape** — because they are not
  the same problem:
  - API-key auth is delivered as an environment variable through the
    backend's credential mechanism, matching the roadmap's "never via
    mounted files or plain env vars" intent.
  - Subscription/OAuth auth is file-based (`~/.claude/.credentials.json`
    confirmed present), so env injection cannot serve it. A new opt-in
    key grants the narrowest possible thing — a **single file,
    read-only** — never a directory, never silently.
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

- `tooling`: a second declarative environment composed into the sandbox
  for tools that are not project dependencies — discovery, resolution
  ordering against the project environment, conflict rules, and the
  reproducibility bar it must meet.
- `credentials`: how a secret reaches a process inside the boundary —
  the two auth shapes, the narrowness requirement for file-based
  credentials, and what is disclosed to the user about the exposure.

### Modified Capabilities

- `config`: `devcroft.toml` gains `[tools]` and the credential opt-in
  key; both validated with the same strictness as existing sections.
- `lifecycle`: `up` refuses a state directory belonging to a different
  project root; `--name` selects an explicit sandbox identity.
- `cli`: `--name` on the commands that resolve a sandbox; `up` discloses
  exactly which credential was exposed and how; `doctor` reports whether
  a declared tooling layer resolves.

## Impact

- Affected specs: new `tooling`, `credentials`; modified `config`,
  `lifecycle`, `cli`.
- Affected code: `src/config/` (two new sections), `src/provider/`
  (resolving a second environment and composing env diffs and store
  grants), `src/policy/` (the credential file grant, with its own origin
  so `policy --render` shows it), `src/lifecycle/up.rs` (project-root
  check, `--name`, credential handoff), `src/bin/devcroft.rs` (`--name`
  plumbing, disclosure output, `doctor`).
- The composition order invariant (`env-provider`'s "Fixed composition
  order") gains a second participant and must stay deterministic — the
  tooling layer's env diff and store grants compose at a fixed,
  documented position, never interleaved by resolution timing.
- The "provider resolution must not widen the policy" invariant applies
  unchanged to the tooling layer: it may add read-only store grants and
  nothing else.

## Success Criteria

- A project declaring an agent CLI in `[tools]` comes up, and
  `devcroft exec -- <agent> --version` runs it inside the sandbox —
  without that agent appearing in the project's own environment
  manifest.
- The agent authenticates inside the sandbox under both auth shapes: an
  API key delivered as an env var, and a subscription credential
  delivered as a single read-only file.
- `policy --render` shows the credential grant with its own origin, and
  `up` prints exactly one line naming the file exposed. A credential is
  never granted implicitly by any other key.
- Two worktrees of the same repo produce two distinct sandboxes with
  `--name`, and produce a clear failure — not silent sharing — without
  it.
- `[tools]` cannot widen the policy: `policy --render` for a manifest
  with a tooling layer differs from one without it only by read-only
  store grants carrying a `tools:` origin.
- A tooling layer that is not reproducible (no lock, or a provider that
  cannot produce one) is rejected at `up`, not accepted with a warning.

## Open Questions

- **Where the tooling layer is declared.** Project-level (committed,
  reproducible for the whole team) is proposed. A user-level layer is
  what the agent case actually wants — my laptop, my agent, not my
  team's — but it reintroduces per-machine variance, the exact property
  devcroft exists to eliminate. Deferring it means a team that wants
  agents declares the agent; that is consistent, and possibly
  unwelcome.
- **Whether the credential opt-in belongs in `devcroft.toml` at all.**
  Committing "grant this token file into the sandbox" to a shared repo
  is a policy statement about every developer's machine. A user-level
  or per-invocation form may be more honest, and depends on the question
  above.
- **What happens when the tooling layer and the project environment
  provide the same binary at different versions.** Fixed precedence is
  the cheap answer; whether the loser is reported, or silently shadowed,
  is a real UX decision this proposal does not settle.
- **macOS parity, unmeasured.** Orca-style desktop fan-out is
  mac-heavy, and the tooling layer, credential grants, and per-root
  identity are all untested there. This is a verification gap, not a
  design unknown — but it could turn into one.

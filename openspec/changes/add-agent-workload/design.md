# Design: add-agent-workload

## Context

See `proposal.md` — Why, including the live probe showing `node` absent
and `~/.claude` unreadable inside a real sandbox.

Constraints that shape everything below:

- **No non-reproducible mode** (CLAUDE.md framing rules). `host` and
  `none` providers are out of scope by design. Any mechanism that puts a
  host binary inside the sandbox is that rejection reopened under a new
  name — and the provider test explicitly warns that "a provider that
  covers language runtimes but leaves the C toolchain to the host has
  smuggled `host` passthrough back in."
- **Fixed composition order** (`env-provider`): environment composition
  is deterministic and inspectable. Adding a second environment must not
  make it timing-dependent.
- **Provider resolution must not widen the policy.**
- **`meta.json` already records `project_root`** — verified. The
  worktree bug is a missing check over data already on disk, not a
  storage change.
- **`~/.claude` is not in `SENSITIVE_PATHS`** — verified. A user can
  grant it today with an ordinary `filesystem.read`. The design question
  is not "can this be done" but "what is the narrowest correct form",
  since the ordinary form grants a whole directory of unrelated state.

## Goals / Non-Goals

**Goals:**

- An agent runs inside the sandbox without appearing in the project's
  own dependency manifest.
- Both auth shapes work, with the file-based one bounded to a single
  file.
- N worktrees of one repo produce N sandboxes, and the current silent
  sharing becomes a loud failure.

**Non-Goals:**

- The listening-socket gap. An agent editing code needs no listener;
  this change is deliberately independent of that blocker so it is
  implementable now.
- Per-sandbox port allocation, service supervision (`add-flox-services`).
- A user-level (non-committed) tooling layer — see Decision 1's cost.
- Any integration code for a specific agent IDE. The SSH surface already
  supports `ProxyCommand`-based connection; nothing here is
  vendor-specific.

## Decisions

### 1. The tooling layer is a declarative environment, declared in the project

Three options were considered for getting an agent runtime inside:

**(a) Tell users to add it to the project's environment manifest.**
Rejected: it versions a personal tool as a project dependency, and
imposes it on every developer on the team whether they use an agent or
not.

**(b) Mount the host's agent binary read-only.** Rejected on the
property that fails, not on taste: a host-linked binary is not part of
any closure, so the sandbox's environment stops being reproducible from
its manifests. This is exactly the `host` passthrough the project has
already rejected, and it would fail the six-criterion provider test on
criteria 2 (restorable lockfile) and 5 (completeness).

**(c) A second declarative, locked environment, composed at `up`.**
Chosen. It keeps every existing invariant intact — resolved host-side in
the trusted phase, captured as an env diff, contributing only read-only
store grants — while separating "what the project needs" from "what I
need in order to work on the project."

**(d) Read what the ecosystem already ships.** Not an option for the
runtime, but its absence from this list was the real gap: nono publishes
signed packs, and `nolabs-ai/claude` exists. Read live
(`docs/prior-art.md`): a pack carries a policy profile and Claude Code
plugin wiring, **no binaries** — so it does not answer the runtime problem.
What it does carry is a technique this change should adopt separately —
a denial-triggered feedback channel that tells the agent *why* a tool call
was refused, which devcroft has the expensive half of already (`why`) and
exposes to nothing.

**Where it is declared: the project, committed.** This is the
uncomfortable half. The agent case genuinely wants a user-level layer —
my machine, my agent, not my team's. But a user-level layer is
per-machine by construction, and per-machine variance is the property
devcroft exists to eliminate. Choosing project-level keeps the
reproducibility claim honest at the cost of making "a team that wants
agents declares the agent" the only supported shape. If that proves
unacceptable in practice, the fix is a user-level layer with its
reproducibility cost stated at `up`, not a quiet host passthrough.

### 2. Credentials split by auth shape, because they are not one problem

The roadmap's stated position — "secret injection delegated to the
backend's credential proxy, never via mounted files or plain env vars" —
was written before this was examined closely, and it only covers half
the ground.

- **Key-shaped auth** fits it exactly: the backend already has a
  credential mechanism that delivers a secret as an environment variable
  with no filesystem grant. Use it.
- **Subscription/OAuth auth is file-based.** Verified:
  `~/.claude/.credentials.json` exists. There is no env var that
  substitutes for it, so "never via mounted files" cannot be satisfied
  for the case that motivates this change. Refusing to support it would
  mean supporting API-key users only — precisely the users who do *not*
  need this, since a key is already injectable today.

So file-based credentials are supported, bounded to the narrowest form
the backend can express: **a single file, read-only.** The backend
supports per-file grants, so granting the containing directory is an
implementation shortcut, not a necessity — the spec forbids it.

**Residual risk, stated rather than mitigated away:** any process in the
sandbox can read an exposed credential, including the project code the
agent is editing. That is unavoidable when the agent must run in the
same boundary as the code under edit. Narrowness (one file), read-only,
and mandatory disclosure at `up` are the mitigations; isolation from the
code under edit is not among them, and the docs must not imply it is.

### 3. Project-root binding: fail, do not auto-suffix

Two worktrees sharing a sandbox is a correctness bug, not an ergonomics
gap. The fix could be automatic — derive a suffix from the root hash so
each worktree silently gets its own sandbox — but that makes the
user-facing handle unpredictable, and `devcroft exec myproj` ambiguous
across checkouts.

Failing loudly and naming `--name` is chosen because it preserves the
property that a sandbox name means one thing, and because fan-out is
usually driven by a script (an IDE recipe, a shell loop) that can supply
a name trivially. The ergonomic cost lands on interactive users creating
a second worktree by hand, who get one clear error once.

The check is nearly free: `meta.json` already stores `project_root`.

### 4. Composition order is fixed, and precedence is stated

With two environments, "which `node` wins" becomes a real question. The
composition order requirement already demands determinism; this change
extends it rather than adding a second, parallel rule. The tooling layer
composes at a fixed documented position relative to the project
environment, and the precedence is part of the spec — not an emergent
property of which resolution finished first.

The remaining UX question — whether a shadowed binary is reported or
silently loses — is left open deliberately (see Open Questions); it
changes no requirement here.

## Risks / Trade-offs

- **The project-level tooling decision may be wrong for the actual use
  case** → Mitigation: it is reversible additively (a user-level layer
  can be added later), and the reproducibility cost is stated up front
  rather than discovered. Recorded as an Open Question in the proposal
  rather than presented as settled.

- **Supporting file-based credentials weakens a stated roadmap
  position** → Mitigation: the position is amended explicitly in
  `docs/decisions.md` with the property that made it unachievable
  (subscription auth has no env-var form), following that file's own
  convention that a rejection whose premise stops holding is revisited
  rather than defended.

- **A narrow file grant still exposes a long-lived token to project
  code** → Mitigation: bounded and disclosed, never implicit. Users who
  cannot accept it should not grant it; the docs say so plainly instead
  of implying safety.

- **BREAKING: a second worktree that worked today now fails** →
  Mitigation: it "worked" by silently serving the wrong code. The error
  names both roots and the fix. Worth a README note, since anyone who
  had two worktrees up was affected without knowing.

- **macOS is unmeasured** → Mitigation: an explicit verification task
  rather than an assumption. Desktop agent fan-out is mac-heavy, so a
  Linux-only result would materially limit this change's value.

## Migration Plan

Additive except for the project-root check. A manifest with neither
`[tools]` nor a credential request behaves exactly as before, and
`policy --render` output for such a manifest is unchanged — which is the
regression test.

The project-root check is the one behavior change on existing setups. It
fails closed, and the failure is actionable, so no data migration is
needed; sandboxes recorded before this change already carry the
`project_root` the check reads.

Rollback is removal of the check and the two new sections; no persisted
state becomes unreadable.

## Open Questions

- Whether a binary shadowed by the tooling layer should be reported at
  `up` or silently lose. Deferrable: the specs require stated
  precedence, not a particular notification, so this changes no
  requirement and no task.
- Which credential shapes beyond the two here are worth naming (for
  example, a helper command that prints a token to stdout). Deferrable:
  additive, and the file/env split covers every agent examined.

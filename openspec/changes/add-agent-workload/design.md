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
- **Fixed composition order** (`env-provider`): the environment resolves
  before restriction, never after. That requirement says nothing about
  two environments (an earlier version of this design read it as if it
  did); how the layer and the project combine is Decision 4, a
  requirement of its own in `tooling`, and it must be deterministic.
- **devcroft reads no user configuration today** — verified 2026-10-29.
  Decision 1 introduces the first.
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
- A project-declared (committed) tooling layer — see Decision 1. A team
  that wants an agent reproducible for everyone already has the answer:
  the project's own environment, option (a).
- Composing the tooling layer's environment beyond `PATH` — see
  Decision 4.
- Any integration code for a specific agent IDE. The SSH surface already
  supports `ProxyCommand`-based connection; nothing here is
  vendor-specific.

## Decisions

### 1. The tooling layer is a declarative environment, declared by the user

Options for getting an agent runtime inside. Three were considered when
this was written; (d) was added on reading what the ecosystem ships, and
(e) only became possible later — the count is left visible rather than
smoothed over, because which options existed *when* is the reason (c) was
chosen:

**(a) Tell users to add it to the project's environment manifest.**
Rejected as *the* answer: it versions a personal tool as a project
dependency, and imposes it on every developer on the team whether they
use an agent or not. It works, though, and is what the eleven agent
samples do (`docs/agents.md`); a team that does want one agent for
everyone should keep doing it.

**(b) Mount the host's agent binary read-only.** Rejected on the
property that fails, not on taste: a host-linked binary is not part of
any closure, so the sandbox's environment stops being reproducible from
its manifests. This is exactly the `host` passthrough the project has
already rejected, and it would fail the six-criterion provider test on
criteria 2 (restorable lockfile) and 5 (completeness).

**(c) A second declarative, locked environment, composed at `up`.**
Chosen. It keeps every existing invariant intact — resolved host-side in
the trusted phase, contributing only a read-only store grant — while
separating "what the project needs" from "what I need in order to work
on the project." What it contributes to a session is its `PATH` and
nothing else (Decision 4).

**(d) Read what the ecosystem already ships.** Not an option for the
runtime, but its absence from this list was the real gap: nono publishes
signed packs, and `nolabs-ai/claude` exists. Read live
(`docs/prior-art.md`): a pack carries a policy profile and Claude Code
plugin wiring, **no binaries** — so it does not answer the runtime problem.
What it does carry is a technique this change should adopt separately —
a denial-triggered feedback channel that tells the agent *why* a tool call
was refused, which devcroft has the expensive half of already (`why`) and
exposes to nothing.

**(e) Run the vendor's own installer in `[hooks] post_create`.** Not
available when this list was written, and now the cheapest thing on it.
`own-sandbox-environment` gave every sandbox a writable `HOME` of its own
(`<project>/.devcroft/<name>/home`), which is precisely what an official
install script wants and what the old `HOME` — the host user's, denied by
the baseline — refused. So the agent installs the way its vendor documents,
into a directory `rm` already owns the lifetime of, and devcroft
reimplements no part of that path.

It does **not** replace (c), and the difference is the tier, not the
convenience: an installer fetches a host-linked artifact, so what it
produces is `artifact` tier at best and unpinned in practice. A hook also
runs *inside* the boundary by the two-phase rule, so it needs its own
`network.allow` entry for the vendor's domain and cannot reach for host
tooling. (c) remains the answer for a developer who wants their agent
pinned, and the same in every project they work on; (a) for a team that
wants one agent for everyone; (e) for one developer who wants it working
this afternoon in one sandbox, and it should be documented as exactly
that rather than sold as equivalent.

**Where it is declared: the user's configuration, not the project
(reversed 2026-10-30, owner's decision).** The first version of this
design chose the project, committed:

> The agent case genuinely wants a user-level layer — my machine, my
> agent, not my team's. But a user-level layer is per-machine by
> construction, and per-machine variance is the property devcroft exists
> to eliminate.

Two things undid that. First, a committed `[tools]` is option (a) in a
different file: the team's repository still names one developer's agent,
and the only thing gained is that a colleague running `flox activate`
without devcroft does not get it. The case that motivates the change —
my laptop, my agent, the team's project — was the one rejected. Second,
the premise was too broad. devcroft's reproducibility claim has always
been about **the project's environment**, and it already tolerates
operator tooling outside the closure: the editor's server is downloaded
into the sandbox, unpinned (`docs/ssh-validation.md`), and nobody reads
that as breaking the project's guarantee. An agent is the same kind of
thing. It is the operator, not a dependency.

So the layer is declared in a user configuration file,
`$XDG_CONFIG_HOME/devcroft/config.toml` (`~/.config/devcroft/config.toml`
when unset, on macOS too), as `[tools] provider = "…"` and `path = "…"`,
naming a directory that holds a flox, devenv, nix flake or devbox
environment of the user's own. `up --tools <path>` names one for a single
invocation and takes precedence over the file. It is never read from
`devcroft.toml`.

The variance is admitted, not hidden, and bounded on three sides:

- **It must still be locked.** A layer with no lockfile is refused at
  `up`, as before. A user layer is per-machine, but on that machine it is
  reproducible.
- **It is recorded.** `Meta` holds the layer's path and a fingerprint of
  its lock; `policy --render` renders from `Meta`, so the policy shown is
  the policy applied, whatever the configuration file says now.
- **It is said.** `up` and `status` name the layer, so "this sandbox has
  more on `PATH` than the project declares" is never discovered by
  surprise.

This is the first user configuration devcroft reads. The policy's inputs
become manifest, provider grants, baseline and the layer, and that is
worth stating plainly rather than letting it arrive as a side effect.

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

**Two things in this decision are unmeasured, and either could overturn
its file half (noted 2026-10-29; no host here has a subscription
login):**

- **A read-only file may not survive a token refresh.** OAuth logins
  refresh their access token and rewrite the credentials file. With a
  read-only grant, an agent may work until the first expiry and then
  fail, in a way that reads as a login problem rather than a sandbox one.
- **The "no env-var form" premise may no longer hold for Claude Code.**
  `claude setup-token` issues a long-lived token for
  `CLAUDE_CODE_OAUTH_TOKEN`, which would make a subscription key-shaped,
  delivered like any other key with no file grant at all. If that holds,
  task 7.1's amendment of "never via mounted files" loses its premise
  for the agent that motivated it.

Both are measured before the file shape is built (task 4.0).

**Where a credential is requested: the user's configuration, with the
layer.** The same reasoning as Decision 1, and more forcefully: a
committed "grant my token file into the sandbox" is a statement about
every developer's machine. The request lives in the user configuration
file beside `[tools]`. The project-level `[env] forward` that already
ships is unchanged and out of scope: it names a variable the user's own
shell must hold, and devcroft never stores the value.

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

### 4. The layer contributes `PATH` entries only, after the project's

With two environments, "which `node` wins" becomes a real question, and
the first version of this design answered it by composing two env diffs
at a fixed position. That was the wrong unit. A nix, devenv or flox
environment's diff carries `CC`, `NIX_CFLAGS_COMPILE`, `NIX_LDFLAGS`,
`PKG_CONFIG_PATH`, `PYTHONPATH` and more; laid over the project's, the
agent's layer would change the project's build.

So the layer contributes **its `PATH` entries and nothing else**:

```
 PATH in a session:
 ┌──────────────────┬──────────────────┬────────────────────┐
 │ project          │ tooling layer    │ devcroft's shell   │   first match wins
 └──────────────────┴──────────────────┴────────────────────┘
 every other variable: the project's alone, then [env.vars] as before
 grants: the layer's store root, read-only, origin tools:<path>
```

- **The project comes first.** A `node` in the layer never shadows the
  project's `node`, so a failure never looks like the project's fault
  because of something the user added for themselves.
- **The layer's hooks never run.** Taking only `PATH`, it is resolved
  through each provider's existing hook-free route (`print-dev-env
  --json`, `shellenv --pure`, `devenv build shell`, the derived flox
  copy), so `fix-provisioning-hooks`' whole problem does not arise for
  it, and its activation script is never carried into the sandbox.
- **Shadowing is shown, not warned.** A name provided by both is not a
  degraded capability, so it does not earn an `up` warning under the
  "exactly one warning" rule. `status` lists the layer's binaries that
  the project's `PATH` shadows. This closes the Open Question the first
  version left.
- **Deterministic by construction.** The layer's entries are taken in
  the order its provider reports them and placed at one position; no
  resolution timing is involved.

**Measured (2026-10-29): the layer needs to contribute PATH entries, not
an env diff.** Composing a second environment's diff would let the
agent's layer change the project's build (`CC`, `NIX_CFLAGS_COMPILE`,
`PKG_CONFIG_PATH`, …), so the question was whether `PATH` alone is
enough. It is, for all eleven agents in `docs/agents.md`. Spike: a devenv
project holding only `jq`, from the same `devenv.lock` as the agent
samples, and a separate `buildEnv` of the eleven agents plus bash,
coreutils, findutils, grep, sed and git, from the same nixpkgs revision.
That is the shape of a devenv profile, and it has no environment of its
own to compose. Each session got `<layer>/bin` appended *after* the
project's `PATH` and nothing else from the layer. All eleven printed
their version, and all eleven reached their own vendor's API through
the proxy and were refused there for a dummy key, the same result as the
matrix, with the agent no longer in the project's environment. No grant
was added: the provider's store grant is the whole `/nix/store` root
(`capture::store_grants`), so a closure-tier layer is already readable.
That also means a layer adds no rule to `policy --render` unless one is
added deliberately, for inspectability.

What this does not show: a layer from a provider whose environment is
not store-backed (artifact tier), a layer whose own tools rely on its
env diff (none of the eleven did, since nixpkgs wraps them with absolute
runtime paths), and macOS.

The grant is rendered anyway. Where the project's provider already
grants the store root, the layer's identical rule adds no access, but it
does make the layer visible in `policy --render` under its own origin;
for a project whose provider grants no store (`swift`), it is the access
the layer needs.

### 5. Only the closure tier serves a layer; it is recorded, and goes stale

- **Closure-tier providers only** — flox, nix, devbox, devenv —
  regardless of the project's own provider. A `swift` project can carry
  a devenv layer; a `swift` layer is refused. An artifact-tier layer is
  host-linked by definition, needs grants of its own
  (`own-policy-baseline`), and is the host passthrough of option (b) one
  step removed.
- **Recorded in `Meta`.** The layer's path and its lock's fingerprint,
  the way the project environment's are. `policy --render` renders the
  layer's grant from `Meta`, never from the configuration file as it
  stands.
- **Stale like the project.** A layer whose lock changed since `up`, or a
  configuration file now naming a different layer, is reported stale by
  `status` and applied by `up --recreate`, exactly as for the project
  environment: the environment resolves once, at `up`.
- **Fleet resolves it once.** A fleet's agents share the user, so they
  share the layer, resolved once per fleet. The minimal root's
  `/workspace` rewrite does not touch it: its `PATH` entries are store
  paths, independent of where any clone lives.

## Risks / Trade-offs

- **A user layer is per-machine** → Mitigation: it must be locked, is
  recorded in `Meta`, and is named by `up` and `status`; the project's
  environment, which is what the guarantee covers, is untouched apart
  from `PATH` entries appended after its own. Two developers' sandboxes
  of one project differ only in what is on `PATH` after the project's
  tools.

- **devcroft gains a user configuration file** → Mitigation: it holds
  the layer and credential requests and nothing that changes how a
  project's own manifest is read. Every other key stays in
  `devcroft.toml`, so "what does this project do" is still answered by
  the repository.

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

Additive except for the project-root check. With no user configuration
file and no `--tools`, every sandbox behaves exactly as before, and
`policy --render` output is unchanged — which is the regression test.

The project-root check is the one behavior change on existing setups. It
fails closed, and the failure is actionable, so no data migration is
needed; sandboxes recorded before this change already carry the
`project_root` the check reads.

Rollback is removal of the check and of the user configuration file's
reader; no persisted state becomes unreadable, though a `Meta` recording
a layer would then carry a field nothing reads.

## Open Questions

- ~~Whether a binary shadowed by the tooling layer should be reported at
  `up` or silently lose.~~ Neither: shown by `status` (Decision 4).
- Turning a configured layer off for one invocation. See the proposal;
  deferred to use.
- Which credential shapes beyond the two here are worth naming (for
  example, a helper command that prints a token to stdout). Deferrable:
  additive, and the file/env split covers every agent examined.

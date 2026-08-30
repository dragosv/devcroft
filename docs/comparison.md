# How devcroft compares

Moved out of the README (2026-08-30) so the README could match a terser,
ecosystem-standard shape — same reason `docs/implementation-log.md` exists.
Nothing here is newer or more authoritative than the README's own short
"Features" and "Known gaps" sections; this is the long-form reasoning behind
them.

## The trade

devcroft isn't a cheaper Docker. It's a bet that most day-to-day
development work doesn't need a full container boundary — it needs a
reproducible environment, a boundary good enough to catch accidents, and
SSH access that works with existing editors, at a marginal cost low enough
to run many side by side on one host.

|  | devcroft | Dev Containers / Docker | flox alone | mise/asdf + manual sandboxing |
|---|---|---|---|---|
| Isolation | Kernel primitives (Landlock/Seatbelt); `process` tier only in MVP — accident protection, not a security boundary (see [threat-model.md](threat-model.md)) | A container boundary, today | None | Whatever you build yourself |
| Editor/SSH access | Native — a real SSH server per sandbox | Native, through the container | No | No |
| Reproducibility | Mandatory — no `host`/`none` fallback (see [decisions.md](decisions.md)) | Optional | Yes | Partial, depends what's pinned |
| Marginal cost per environment | Low — a shared Nix/flox store, no separate rootfs or guest kernel | Higher — image layers, often a VM on macOS | None | None |

That trade makes sense for fleets of coding agents, many parallel projects
on one host, or local CI — not for running code you don't trust at all,
where a real container or VM boundary is still the right call.

## devcroft vs `nono-cli`

The closest relative isn't Docker — it's `nono-cli`, the CLI shipped by
[nono](https://github.com/nolabs-ai/nono), the same library devcroft
consumes for its own enforcement (`use-nono-library`). Worth answering
directly, since "why not just add this to nono-cli" is the obvious
question once that dependency is visible.

`nono-cli`'s own command surface answers it: `run`, `shell`, `wrap` —
apply a capability set, exec a command, the sandbox lasts as long as that
one invocation. `ps`/`attach`/`detach` manage sessions on the same
machine the CLI runs on. `profile` is the sandboxing policy itself,
authored directly or pulled from a signed pack registry. There is no
manifest, no lockfile, no notion of a project's toolchain — `nono run`
assumes whatever you're sandboxing is already on `PATH`.

devcroft needs three things nono-cli has no model for at all, not just a
thinner version of:

- **Provisioning before restriction.** Two-phase execution (CLAUDE.md):
  resolving a reproducible toolchain — flox/nix/devbox, a manifest plus
  its lock — has to happen host-side, *before* any sandbox exists, then
  get folded into the policy the sandbox applies. `nono wrap` has nothing
  upstream of the sandbox; it assumes the environment is already there.
- **A persistent supervisor, not a per-invocation wrapper.** `up` starts
  a keeper once; `exec`, `shell`, and SSH all attach to the *same* running
  sandbox afterward, until `down`. `nono run`/`wrap` restrict one process
  tree for one command's lifetime — there is no "start once, connect many
  times" concept, and no daemon to connect to.
- **A real SSH server as the primary interface.** devcroft's sandbox is
  meant to be pointed at from an existing editor, possibly on another
  machine. `nono-cli`'s `attach`/`detach` are local session management,
  not a network-facing protocol server.
- **Long-running services, and a port namespace to put them in.** A dev
  environment is not just a command — it's Postgres, Redis, a queue, each
  needing to start with the sandbox and stop with it. `nono-cli` has no
  service concept at all; it sandboxes a process and exits. More
  importantly, N sandboxes of the same project all declare the *same*
  committed port, so they need a private port table each to avoid
  colliding on it — a property only containers and VMs otherwise provide,
  and the capability that most clearly separates a dev-environment tool
  from a general sandboxing CLI: `nono run` has nowhere to put a second
  Postgres. devcroft does, for the common case: a sandbox with services
  or ports and no outbound network gets its own network namespace, so N
  of them bind the identical port with nothing colliding
  (`CompiledPolicy::wants_network_isolation`,
  `tests/network_isolation_e2e.rs`) — *and* keeps filtered egress inside
  that namespace, reached through a unix socket that crosses it
  (`tests/isolated_egress_e2e.rs`). Both properties at once is the
  combination an agent needs and the one shape a general sandboxing CLI
  has no model for.

**This is not "host access versus none" — both tools grant host paths,
declared differently.** `nono-cli`'s `-a`/`-r`/`-w` flags carve permissions
out of the host filesystem per invocation, ephemeral and unversioned —
its whole model assumes the toolchain is already there and your job is to
open specific doors into it. devcroft's baseline grants none of that: a
closure-tier provider's build gets only the project root, `/tmp`, and the
provider's own store — measured live, `/usr/bin/gcc`/`/usr/lib` denied
even when installed on the host (`docs/decisions.md`). But the manifest
*can* grant arbitrary host paths too — `filesystem.allow`/`read` accept
absolute and `~` paths, not just project-relative ones — so the real
difference isn't capability, it's where the grant lives: a CLI flag typed
fresh each time, versus a line in a committed `devcroft.toml` that shows
up with a `manifest:` origin in `policy --render` and warns on `~/.ssh`.
One is a decision made per session; the other is a decision made once and
reviewed like any other code change.

None of that is nono-cli doing its job worse — it's a different job.
nono's own stated design is a small, embeddable, auditable capability
library: "no escape hatch", a narrow surface, one thing done
irreversibly. Bolting environment resolution, an SSH server, service
supervision, and fleet orchestration onto it would turn a security
primitive into an opinionated dev-environment platform, inside the trust
boundary of every *other* consumer of that library — a much bigger ask
than the dependency tail `use-nono-library` already weighed once as its
one accepted objection. The dependency direction devcroft actually uses —
consuming nono as a mechanism and building policy and orchestration on
top — is the right one; reversing it would force nono's other consumers
to carry devcroft's opinions whether they want a dev-environment tool or
not.

## Other sandboxes devcroft has read

`sandlock` and bubblewrap sit where `nono-cli` does — tools that confine a
command rather than provision an environment — so the reasoning above
applies to them unchanged, and neither is a backend candidate. What
devcroft has taken from each (a seccomp handoff sequence, a Landlock
scoping mode it already had access to, a mount-plan reference) is recorded
in [prior-art.md](prior-art.md), so an idea's origin does not get lost and
re-litigated.

## How coding-agent products provision environments today

Three patterns dominate, plus one that isn't provisioning at all. All four
exist to solve the same problem: the environment gets built once at fleet
scale, not once per agent — nobody can afford N × `npm install`/`cargo
build` from a cold start.

**Snapshot / golden disk** (Cursor cloud agents, Devin). Set the
environment up once — by hand, or by letting an agent do it interactively
— then save the resulting disk state. Cursor runs the install script from
`.cursor/environment.json` once per [Build](https://cursor.com/docs/cloud-agent/builds),
in the background rather than on every agent start; a successful Build
becomes the disk state every new agent starts from, with config resolved
in order (repo `.cursor/environment.json` → personal env → team env). The
known failure mode is drift: a staleness threshold (24h by default) makes
an agent pull the latest default-branch commit past that age, otherwise it
reuses whatever commit the Build was made from — and the snapshot itself
stays opaque; nothing says what's actually in it three months on.

**Polyglot base image + cached setup script** ([OpenAI Codex
cloud](https://developers.openai.com/codex/cloud/environments)). Every
container starts from `codex-universal`, one Ubuntu image with runtimes
preinstalled for eight languages (Python, Node, Rust, Go, Ruby, PHP, Java,
Swift), pullable locally to test the setup script before it runs in the
cloud. Flow: clone the default branch → run the setup script → cache the
resulting container state for up to 12 hours; a new task checks out the
requested branch against that cache and can run an optional maintenance
script, useful when the cache predates the branch. The catch that trips
people up: the setup script runs in a Bash session separate from the
agent's own, so a plain `export` doesn't persist past it.

**CI as the environment** ([GitHub Copilot coding
agent](https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/customize-the-agent-environment)).
The agent gets an ephemeral environment provisioned by GitHub Actions
itself, customized through a `copilot-setup-steps.yml` job with a fixed
name, where only `steps`/`permissions`/`runs-on`/`container`/`services`/
`snapshot`/`timeout-minutes` are honored. The upside: existing CI
definitions get reused instead of a second build system getting invented.
GitHub's own stated reasoning for why this exists rather than leaving the
agent to figure it out: it can discover dependencies itself by trial and
error, but that's slow and unreliable given how non-deterministic LLMs
are, and for private packages it can be outright impossible.

**Local fan-out: worktrees** (Claude Code). No provisioning at all — the
environment is the laptop, shared. `--worktree`, plus `isolation:
"worktree"` on subagents, gives each agent its own directory and branch
without manual git plumbing. In practice 4–5 worktrees is the ceiling on
one laptop; past that, agents move to a remote machine and branches get
pulled back. Isolation here is filesystem-view only, not environment: port
collisions, a shared local database, a shared `target/`, and a `.env` that
doesn't get copied into the new worktree are all still there.

**What none of the four actually solve:**

- **Stateful services.** Postgres/Redis either go into the image as a
  `services:` block, or every agent gets its own port/schema by hand —
  nobody has this elegantly solved.
- **Secrets.** Injected at runtime, deliberately *not* baked into the
  snapshot (Cursor's own docs point at its Secrets tab over a
  `.env.local` captured into a snapshot).
- **Network.** Deny-by-default once setup finishes, or the agent has an
  exfiltration channel open the whole time it runs. Copilot and Codex
  both expose a configurable firewall specifically for the agent phase.
- **Verification.** The rule that shows up everywhere in practice: setup
  ends with a command that *proves* the repo is ready (`cargo test
  --no-run`, `bun run validate`), with an explicit "stop and report on
  failure" instruction — otherwise the agent spends an hour debugging
  perfectly fine code in a broken environment.

## Where devcroft differs

All four patterns above treat the environment as an **imperative
artifact** — a script plus a snapshot — which is opaque, vendor-specific,
and drifts. The declarative alternative this project bets on — a
flox/nix manifest plus its lock — makes the snapshot redundant: the
manifest *is* the snapshot. It's bit-reproducible, it's already the thing
checked into the repo, and instantiating eight identical environments
costs one cached build, not eight image builds.

That's the actual differentiator from "yet another Docker wrapper" — not
isolation (Landlock/Seatbelt are commodity at this point; [this list of
sandboxing tools](https://gist.github.com/wincent/2752d8d97727577050c043e4ff9e386e)
alone has 20+ entries), but **reproducibility plus the marginal cost of
instantiation**. The local-worktree pattern above is both the most-used
fanout mechanism and the worst-served: it gets git plumbing for free and
nothing for the environment underneath it. A `devcroft.toml` manifest
with a lock is exactly what that workflow is missing — giving every
worktree an identical, isolated, auto-provisioned environment off the
same lock, with ports allocated instead of collided and services isolated
instead of shared, is a natural next layer on this model. It isn't in
MVP's closed command surface today (see the README's Features section) —
worth naming as direction, not claiming as delivered.

# devcroft

Isolated, reproducible development environments built on OS-level sandboxing
— kernel primitives instead of containers or VMs — each reachable over SSH.

Many environments, including fleets of coding agents, run in parallel on one
host at near-zero marginal cost: they share a single content-addressed Nix
store while each stays behind its own kernel-enforced boundary. Because every
sandbox speaks SSH, existing editors work unchanged.

devcroft implements no isolation itself. It is a policy compiler, a
supervisor, and an SSH endpoint over existing sandbox backends.

## How this compares

devcroft isn't a cheaper Docker. It's a bet that most day-to-day
development work doesn't need a full container boundary — it needs a
reproducible environment, a boundary good enough to catch accidents, and
SSH access that works with existing editors, at a marginal cost low enough
to run many side by side on one host.

|  | devcroft | Dev Containers / Docker | flox alone | mise/asdf + manual sandboxing |
|---|---|---|---|---|
| Isolation | Kernel primitives (Landlock/Seatbelt); `process` tier only in MVP — accident protection, not a security boundary (see Limitations) | A container boundary, today | None | Whatever you build yourself |
| Editor/SSH access | Native — a real SSH server per sandbox | Native, through the container | No | No |
| Reproducibility | Mandatory — no `host`/`none` fallback (see [docs/decisions.md](docs/decisions.md)) | Optional | Yes | Partial, depends what's pinned |
| Marginal cost per environment | Low — a shared Nix/flox store, no separate rootfs or guest kernel | Higher — image layers, often a VM on macOS | None | None |

That trade makes sense for fleets of coding agents, many parallel projects
on one host, or local CI — not for running code you don't trust at all,
where a real container or VM boundary is still the right call.

### How coding-agent products provision environments today

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

### Where devcroft differs

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
MVP's closed command surface today (see Status) — worth naming as
direction, not claiming as delivered.

## Status

**Working today, on Linux and macOS.** The MVP is implemented: a manifest
compiles to a kernel-enforced policy, a keeper process applies it to itself,
and sessions, hooks, services and SSH all run inside that boundary.

| | |
|---|---|
| Environment providers | flox, nix flakes, devbox — all closure-tier |
| Isolation | one tier, Landlock (Linux) / Seatbelt (macOS) |
| Access | `exec`, `shell`, and a real SSH server per sandbox |
| Services | provider-declared, supervised per sandbox |
| Egress | domain allowlists, enforced through a resident proxy (`nono-proxy`) |
| Policy | deterministic, inspectable via `policy --render` and `why` |

Verified end to end against real tooling in this repo's own devcontainer —
`cargo test` self-skips where a provider or kernel feature is missing rather
than passing vacuously.

**Command surface** (closed for the MVP): `init`, `up`, `down`, `rm`,
`status`, `logs`, `ps`, `shell`, `exec`, `ssh`, `proxy`, `ssh-config`,
`policy`, `why`, `doctor`.

### Known gaps

Published rather than hidden — each is a deliberate scope decision, not an
oversight:

- **No inter-sandbox process visibility separation.** Two sandboxes on one
  host can see each other's processes. Fixed by `add-linux-agent-fleet`'s
  per-agent PID namespaces, which are not yet built.
- **Domain filtering is enforced on Linux; unverified on macOS.** The Linux
  path is a kernel gate plus a resident proxy. Whether Seatbelt enforces the
  equivalent as strictly is untested — this project has no macOS host to
  measure it on, and does not ship a security claim it hasn't measured.
- **No cgroup resource limits.** One runaway build can starve the host. Also
  fleet's subject.
- **Provisioning runs on the host** — with one exception now closed. Resolving
  a provider environment happens before any boundary exists. For flox, whose
  `[hook].on-activate` is arbitrary project shell, devcroft now materializes
  from a derived hook-free copy of the environment and runs the hook *inside*
  the sandbox instead, so no project code executes unconfined. The rest of
  provisioning still runs host-side; `sandbox-provisioning` is the change that
  moves it. An upstream request that would make the flox split a supported
  contract rather than devcroft's inference is drafted at
  [docs/flox-confined-activation-issue.md](docs/flox-confined-activation-issue.md).

### In flight

Run `openspec list` for live progress. The larger ones:

- **`add-linux-agent-fleet`** — N coding agents on one host, each with its own
  workspace, service stack, network namespace and resource budget. The
  per-agent network namespace is implemented; that is what lets every agent
  bind the same `5432` without collision.
- **`sandbox-provisioning`** — move provider resolution inside a boundary.
- **`add-agent-workload`** — how an agent's own tooling and credentials are
  declared, rather than assumed present on the host.
- **`add-agent-interaction`** — what happens when an agent stops and needs a
  decision. Today nothing does: a blocked agent is indistinguishable from a
  busy one, which is fine at N=1 and defeats the point of a fleet.
- **`add-port-allocation`** — distinct ports for parallel sandboxes that share
  the host's loopback (the non-fleet case).

### History

The blow-by-blow — what was built, what turned out to be wrong, and what the
corrections cost — is in
[docs/implementation-log.md](docs/implementation-log.md). It lived here until
it was 376 lines long and had crowded out everything above.


## Limitations

devcroft has one isolation tier: Landlock or Seatbelt applied to a process
tree, with the environment's own policy around it. **This is accident
protection, not a security boundary** — the full host kernel syscall surface
stays reachable from inside, so a kernel bug is an escape. It contains an agent
that misbehaves, deletes the wrong directory, or fights another agent for a
port; it does not contain code written to escape.

If you need that, run devcroft inside a VM. That is the supported answer rather
than a deflection: it is already how the macOS path works. See
[docs/threat-model.md](docs/threat-model.md) for which use case each of those
actually backs.

### We built a hardened tier and removed it

An earlier version had a second tier backed by gVisor. It worked — full
environment startup, `exec`, SSH and services, verified against real tooling.
It was removed anyway (`remove-gvisor-backend`; the code is recoverable at the
tag `gvisor-backend-last`), for three reasons worth stating:

**Landlock cannot confine anything that builds its own filesystem view.**
`runsc` needs `mount()`; Landlock has no hook for `mount()` at any ABI version,
so the two fail together with `EPERM` under any ruleset, however permissive —
confirmed by elimination, including one granting `/` full read-write. Composing
the tiers was structurally impossible, not merely fiddly.

**The middle was squeezed.** Below it, the process tier is cheaper and matches
what devcroft is for. Above it, a VM is stronger and already required
elsewhere. A tier more complicated than the first and weaker than the second
has to earn its place, and every new feature would have been designed twice.

**Rootless operation cost it the property it was chosen for** — with a caveat
worth keeping, because the obvious version of this claim is wrong. `runsc`
rejects its sandboxed-network mode under `--rootless`, so the tier could never
give fleet a per-instance netstack. But port separation there never came from
gVisor: it came from the network namespace devcroft requested in its own OCI
spec, so a deny-default hardened sandbox did get its own loopback and did *not*
collide. Only granting egress shared the host's port space. A tier whose port
isolation disappears the moment you allow network access still cannot carry
fleet — but "it structurally lacks it" would be overstating it.

What we kept: the backend abstraction, so a future backend is an addition
rather than a rewrite; a written set of criteria any candidate has to meet; and
three integration defects that only appeared when real toolchains ran — mount
destinations needing to exist in the bundle beforehand, `root.path` needing to
be absolute for gVisor's symlink-escape guard, and an argument separator
`runsc exec` rejects. None were in any documentation. Budget for that class of
defect in any sandbox integration.

Known gaps, published rather than hidden:

- **No PID/mount/network namespace separation between sandboxes**
  ([design.md](openspec/changes/add-mvp-core/design.md) Decision 5) — still
  true structurally: Landlock hides nothing, so sandboxes share the host's
  raw process and network namespaces. What this means in practice turned
  out narrower than Decision 5 originally assumed, though: on a Landlock
  **ABI V6** host (`doctor` reports the ABI level; this repo's own
  devcontainer is V6), `tests/process_tier_landlock_boundaries.rs` proves
  live that a sandboxed process can neither `kill()` nor read
  `/proc/<pid>/*` for a process outside its own sandbox — V6's signal-
  scoping LSM hook and the default-deny filesystem policy (which covers
  `/proc` like any other ungranted path) close both, even with no PID
  namespace to enforce it structurally. This is kernel-version-dependent,
  not a blanket guarantee — older kernels without ABI V6 would plausibly
  still allow it, and `doctor`'s ABI line is how to know which regime a
  given host is in. What the missing namespace separation still means
  regardless of ABI version: two sandboxes binding the same port (e.g.
  both running a dev server on 3000) still race for it with `EADDRINUSE`,
  since Landlock has no hook for that at all. There is no conflict
  detection; reach a sandbox's services through SSH's `-L` forwarding
  rather than assuming host ports are exclusive to it.

  **This bites hardest on exactly the fan-out flow devcroft targets**, and
  is no longer moot now that services ship: `devcroft.toml` is committed,
  so every git worktree of a repo declares the *same* port, and two
  sandboxes each starting Postgres on 5432 collide. It is tier-dependent
  in a way worth knowing — at `hardened` with a **deny-default** network
  the sandbox already gets its own network namespace from the OCI spec, so
  the committed port works unchanged in all N; the collision is real at
  `process`, and at `hardened` when egress is granted (which shares the
  host's namespace). The fix is `add-port-allocation`, which pairs with
  `add-agent-workload`: that change gives N worktrees distinct sandbox
  *names*, without which they never get as far as needing distinct ports.
- **~~`network` blocking also blocks *listening* sockets~~ — FIXED, and
  the original diagnosis was wrong.** A deny-default policy does still
  deny `bind`/`listen` by itself, but this was published as a gap in the
  policy model — "there is currently no way to express *no outbound
  access, but I can still run my dev server*" — and that was false. nono's
  profile schema has always carried an `open_port` field; devcroft simply
  never emitted it. `[network].ports` now does: `default = "deny"` plus
  `ports = [3000]` binds `127.0.0.1:3000` while egress stays filtered and
  ungranted ports stay denied, verified end to end in
  `tests/network_ports_listen.rs`. The `allow`-everything workaround is no
  longer required, and the VS Code Remote-SSH blocker in
  [docs/ssh-validation.md](docs/ssh-validation.md) should be re-tested
  against this key rather than assumed. Worth recording how long a wrong
  claim survived unchecked: it was repeated across the docs and treated as
  an architectural constraint, and one `nono profile schema` invocation
  refuted it.
- **Network filtering is platform-dependent; on Linux it's less "purely
  cooperative" than first assumed.** `add-egress-proxy` shipped a real,
  enforced domain filter on Linux (Landlock `NetPort` gates every
  `connect()` except to a resident proxy, which decides by hostname) —
  `docs/decisions.md`'s and this section's older framing, that domain
  filtering everywhere was merely cooperative, no longer describes
  Linux. Whether macOS Seatbelt enforces the equivalent
  `NetworkMode::ProxyOnly` gate as strictly, or only adds a permissive
  rule without narrowing anything else, is **unverified** — the pinned
  library's own doc comment for the macOS output reads as a scoped allow
  rule, which would argue for "enforced" under Seatbelt's default-deny
  model, but this project has no macOS host to measure it live on, and
  this project does not ship a security claim it hasn't measured. The
  degraded-on-macOS warning stays on until someone can check. On Linux,
  the original assumption here was that a process could always bypass a
  domain allowlist with a raw socket straight to an unresolved IP.
  `tests/process_tier_landlock_boundaries.rs` tested that directly and
  found it doesn't hold on this host: `policy --render` shows
  `network.block: true` even with an allowlist set, and a raw socket to
  an IP with no relation to any allowed domain gets a kernel-level
  `Permission denied` — nono's own Landlock network scoping, not an
  unenforced proxy hint the socket simply never talks to. Left genuinely
  open (untested, not claimed as safe): whether the *allowed* domain's
  own resolved-IP scope is wider than intended — a different service on
  the same allowed IP, or DNS-rebinding-shaped tricks.
- **No cgroup resource limits.** A runaway build in one sandbox can affect
  the whole host — nothing today caps CPU or memory per sandbox. Planned:
  cgroup v2 scope units per keeper on Linux; no macOS equivalent exists.
- **A `filesystem.allow` grant for a path that doesn't exist yet is
  silently dropped**, while `policy --render` still shows it as granted
  with its `manifest:` origin. The backend ignores grants whose target is
  missing when the profile is applied, so the rendered policy is not the
  policy in force — the one gap here that contradicts an invariant
  ("deterministic and inspectable", "degraded capabilities are surfaced,
  never silent") rather than just missing a feature. Create the directory
  before `up` as a workaround. Found during task 6.5.
- **`devcroft up` on a flox project runs that project's activation hook
  on your host, outside the sandbox.** Provider resolution happens
  host-side before any restriction exists — that is how the toolchain
  gets materialized — and a flox manifest's `[hook].on-activate` is
  arbitrary shell that `flox activate` runs. Measured against flox
  1.14.0: no mode suppresses it. devcroft detects the hook and `up`
  prints one warning, because it cannot prevent it.
  **So `devcroft up` on a repository you have not read is running its
  code**, the same as typing `flox activate` there yourself. The nix
  provider does *not* have this: it reads the dev shell's environment as
  structured data and never evaluates the `shellHook`.
- **Zed's remote server connects and transfers but does not start.** Its
  forked daemon exits without logging; not yet attributed to devcroft.
  Zed also needs five separate `$HOME` grants, one of which is the local
  editor's own data directory. VS Code and Cursor are unaffected. See
  [docs/ssh-validation.md](docs/ssh-validation.md).
`docs/decisions.md` has the falsifiable "why not X" reasoning behind most of
these; the ones above are gaps in what's actually built, not design
decisions.

| | |
|---|---|
| [openspec/changes/add-mvp-core/](openspec/changes/add-mvp-core/) | The MVP — proposal, design, tasks, 7 capability specs |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [docs/ssh-validation.md](docs/ssh-validation.md) | SSH client/editor validation matrix (task 6.5) — OpenSSH, rsync, VS Code Remote-SSH and Cursor validated against a live sandbox; Zed connects and transfers but its server does not start |
| [CLAUDE.md](CLAUDE.md) | Architecture invariants and repo conventions |
| [samples/flox-rustup-sample/](samples/flox-rustup-sample/) | A real, verified flox + rustup + devcroft project — and the 4 real sandboxing/toolchain frictions found building it |
| [samples/flox-clap-sample/](samples/flox-clap-sample/) | A clap-derive CLI sandboxed the same way — plus what changes once a sample has real crates.io dependencies |
| [samples/devbox-citytime-sample/](samples/devbox-citytime-sample/) | A third `env.provider`, devbox — and why it has no host-side hook to fetch dependencies in, unlike the other two |

```sh
openspec list             # active changes and task progress
openspec validate --all   # validate delta specs
```

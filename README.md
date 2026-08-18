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

**MVP implementation underway — 23/25 tasks.** A `..` path-traversal gap in
task 2.1's `filesystem.allow`/`read`/`deny` validation, and a task 3.2
reproducibility gap where flox activation inherited whoever's shell ran
`up` (personal `PATH`, ad hoc env vars) instead of a fixed environment,
were both found and closed along the way — see the git history for each
fix. The fd-passing keeper trick
(spike binary, task group 1) is proven on both Linux/Landlock and
macOS/Seatbelt; the config/policy compiler, the environment provider layer
(`flox` resolution, task group 3), the keeper's spawn protocol (control
socket, session registry, pty allocation — task 4.1), the supervisor
(`up`/`down`/`rm`, idempotent with crash recovery and `--recreate` — task
4.2), read-only sandbox introspection (`status`/`logs`/`ps` — task 4.3), and
sessions — task group 5 in full: one-shot `exec` with exit-code
propagation, cwd mapping, and signal forwarding (5.1); interactive `shell`
with a real pty, resize propagation, and a `$SHELL`-then-`/bin/sh` fallback
(5.2); and auto-up (`exec`/`shell` bring a cold sandbox up themselves unless
`--no-up`, 5.3) — are implemented and tested end to end against real `nono`
and `flox`. Task group 6 (SSH endpoint) is now complete except for its
cross-editor validation matrix (6.5, nearly done — OpenSSH and rsync are
validated by real end-to-end tests, VS Code Remote-SSH and Cursor are
validated by real manual connections against a live sandbox, only Zed
remains (no CLI to drive it here) — see
[docs/ssh-validation.md](docs/ssh-validation.md)): an SSH server (russh)
embedded in the
keeper on a second unix socket, mode 0600 in the state dir's mode 0700, bound
host-side and fd-passed the same way the control socket is, with publickey
auth against the devcroft client keypair and a fresh ephemeral host key per
`up` (6.1); `devcroft proxy <name>.devcroft` and `devcroft ssh-config
[--write]` (6.2); and full channel support (6.3/6.4) — exec, pty/shell with
resize and an env allowlist (`TERM`/`LANG`/`LC_*`), exit status, the `sftp`
subsystem (also what modern `scp` speaks by default), and `-L` direct-tcpip
forwarding gated by nothing devcroft-specific — it just lets the sandbox's
own network restriction accept or reject the target, same as every other
syscall the keeper makes. All of it is tested against the real `ssh`/`scp`/
`sftp` CLIs through a real `devcroft proxy` subprocess, not just a russh test
client. Task group 7 (CLI polish & release) is well underway: `devcroft
init` detects an existing flox environment or a bare single-ecosystem
toolchain pin (`rust-toolchain.toml`/`.nvmrc`/`.python-version`) and
generates a minimal manifest without ever overwriting one without
`--force`; its default sandbox name (the directory slug) is disambiguated
with a short path-derived suffix only on a real collision against another
project's already-existing state — e.g. two unrelated projects both named
`api` — so the common case keeps the plain slug and only a genuine clash
gets a suffix; `devcroft doctor` reports backend presence/version-range, kernel
sandboxing capability, the provider binary, `ssh-config` managed-section
state, and (when a manifest is discoverable) which of its aspects would be
degraded on this host, with every `FAIL` naming its fix (7.1). The rest of
the command surface is wired up too, each with the stable 0–5 exit codes
and layer-named errors the cli spec's error contract requires: `up`
(idempotent, `--recreate`), `down`, `rm`, `status`, `logs`, `ps`, `policy
--render`, `why --path`/`--host`, and `ssh` (execs a real system `ssh` with
the right options pre-filled). Destructive operations (`rm`, `up
--recreate`) refuse to run non-interactively without `--yes` (7.2). Two
sandboxes now have end-to-end coverage
running side by side with disjoint state and independently-enforced
policy, and a keeper survives a freeze/resume cycle (`SIGSTOP`/`SIGCONT`
on the keeper pid, the realistic proxy for host suspend/resume available
in this environment) with the next command transparently confirming
health rather than assuming it (7.3).

**Post-MVP:** `add-nix-provider` is implemented — nix flakes as a second
`env.provider` value alongside flox, same closure tier, same contract
(`Provider` trait, host-side activation capture, store grants, staleness
fingerprinting). `init` and `doctor` both learned about it; see
[samples/nix-flake-sample](samples/nix-flake-sample/) for a working
example and `openspec/changes/add-nix-provider/` for the full spec. This
also closed a real, pre-existing gap that predated nix entirely: `policy
--render`/`why` never showed *any* provider's store grants before this
(`Origin::Provider` existed since MVP with no caller) — fixed for flox
and nix alike.

**`add-hardened-tier`/`add-gvisor-backend` are implemented** (17/17 and
28/28 tasks) — the `hardened` tier's first concrete backend, gVisor. The
manifest's `[sandbox].isolation` key, the `SessionBackend` trait
`lifecycle::up` dispatches sessions through, and the `gvisor` module
(OCI bundle synthesis from the same `CompiledPolicy` the process tier
compiles, `runsc` command assembly, a Landlock profile applied to the
Sentry process as defense in depth, `doctor` diagnostics, a pinned
`runsc` install in the devcontainer) are all implemented, unit tested,
and covered by real-tooling integration tests
(`tests/gvisor_hardened_e2e.rs`, `tests/hardened_tier_ssh_parity.rs`)
that self-skip wherever `runsc` isn't functionally usable, the same
convention every other real-tooling test in this suite already follows.
One correction along the way, made before any code shipped against the
wrong assumption: an earlier draft leaned on gVisor's per-sandbox
netstack to close the listen-socket gap below for free, but `runsc`
rejects that mode outright under `--rootless`, and devcroft runs
unprivileged everywhere by design — so the hardened tier shares the
host's network namespace exactly like `process` does, and does **not**
close that gap either (see the note below).

What isn't verified: an actual live session round-trip and SSH handshake
against a *running* gVisor sandbox. This repo's own devcontainer ships a
real, pinned `runsc` (task group 8's install), but could not create
unprivileged user namespaces (`unshare --user` failed `EPERM`) —
diagnosed as the container runtime's default seccomp profile blocking
`clone(CLONE_NEWUSER)` for a process without effective `CAP_SYS_ADMIN`,
not the more commonly cited `kernel.unprivileged_userns_clone` sysctl
(that path doesn't exist on this kernel at all). `devcroft doctor`
reported this live and correctly: `[FAIL] gvisor-backend: ... the
systrap platform does not work on this host (... fork/exec
/proc/self/exe: operation not permitted)`, and
`tests/gvisor_hardened_e2e.rs` self-skipped with the same finding rather
than claiming coverage it didn't have. `.devcontainer/devcontainer.json`
now sets `"runArgs": ["--security-opt", "seccomp=unconfined"]` to lift
that block — a deliberate reversal of this file's earlier "no
security-opt relaxations" stance for Landlock, recorded in that file's
own comment along with why the narrower `--cap-add=SYS_ADMIN` doesn't
work here (the devcontainer's `remoteUser` is non-root, and a non-root
process's effective capabilities stay empty regardless of the
container's granted set). **Unverified**: the diagnosis (seccomp
blocking `clone(CLONE_NEWUSER)` absent effective `CAP_SYS_ADMIN`) is
confirmed directly against the running container's own
`/proc/self/status`, but the fix itself is not — no docker socket is
reachable from inside a running devcontainer to drive a rebuild from
this session, so whether `seccomp=unconfined` actually clears the EPERM
is untested. The next rebuild is what `devcroft doctor`'s gvisor-backend
check and `tests/gvisor_hardened_e2e.rs` will either confirm or correct.
Before `runsc`
was installed here at all, a real release binary was fetched out-of-band
and driven through the actual code path by hand against a real
nix-flake project, which caught and fixed four real bugs before any of
them could ship — a Landlock ruleset that denied `runsc` its own
`execve`, a missing grant for the `/proc/sys` tunables `runsc`'s own
preflight reads, missing grants for the OCI bundle and `runsc`'s
`--root` state directory, and a `-d` flag that doesn't exist (`-detach`
does) — and reached exactly the userns wall diagnosed above, no further.
Everything upstream of that wall (bundle synthesis, Landlock, `runsc
run` argument assembly) is now real-world tested; the wall itself,
`-detach` actually detaching, signal propagation into a sandboxed
process, and the Landlock ruleset surviving into a started Sentry remain
unconfirmed absent a host (or a deliberately relaxed container) that
permits unprivileged userns creation.

## Limitations

devcroft's default (and only fully implemented) tier, `process`, is
Landlock or Seatbelt applied to a process tree. **This is accident
protection, not a security boundary** — the full host kernel syscall
surface stays reachable from inside, so a kernel bug is an escape. A real
boundary is the `hardened` tier (gVisor via `add-gvisor-backend`, or
LiteBox; see
[openspec/changes/add-hardened-tier/](openspec/changes/add-hardened-tier/)),
implemented but not yet verified end to end against a live sandbox — see
the Status section above for exactly where that verification stops.
Every isolation claim in this README and in `devcroft`'s own output is
scoped to `process` unless said otherwise.

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
  rather than assuming host ports are exclusive to it. **Note this is
  currently moot under the default policy** — see the listening-socket
  gap below, where neither sandbox can bind in the first place.
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
  cooperative" than first assumed.** macOS Seatbelt genuinely cannot
  enforce domain-level allowlisting without a cooperative proxy —
  `doctor` and `up` name that degradation once, rather than silently
  granting broader network access than the manifest asked for. On Linux,
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

```sh
openspec list             # active changes and task progress
openspec validate --all   # validate delta specs
```

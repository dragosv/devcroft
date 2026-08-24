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

**`add-hardened-tier`/`add-gvisor-backend` are implemented and now
verified live**, end to end, against a real rootless `runsc` (17/17 and
28/28 tasks). The manifest's `[sandbox].isolation` key, the
`SessionBackend` trait `lifecycle::up` dispatches sessions through, and
the `gvisor` module (OCI bundle synthesis from the same `CompiledPolicy`
the process tier compiles, `runsc` command assembly, `doctor`
diagnostics, a pinned `runsc` install in the devcontainer) are all
implemented, unit tested, and covered by real-tooling integration tests
(`tests/gvisor_hardened_e2e.rs`, `tests/hardened_tier_ssh_parity.rs`,
`tests/hardened_services_wiring.rs`) that self-skip wherever `runsc`
isn't functionally usable, the same convention every other real-tooling
test in this suite already follows. One correction along the way, made
before any code shipped against the wrong assumption: an earlier draft
leaned on gVisor's per-sandbox netstack to close the listen-socket gap
below for free, but `runsc` rejects that mode outright under
`--rootless`, and devcroft runs unprivileged everywhere by design — so
the hardened tier shares the host's network namespace exactly like
`process` does, and does **not** close that gap either (see the note
below).

**Getting a real `runsc` running here took two fixes, landed in
sequence, each confirmed against the running container rather than
assumed:** first, `.devcontainer/devcontainer.json` sets
`"runArgs": ["--security-opt", "seccomp=unconfined"]` — the container
runtime's default seccomp profile was blocking `clone(CLONE_NEWUSER)`
for a process without effective `CAP_SYS_ADMIN` (not the more commonly
cited `kernel.unprivileged_userns_clone` sysctl, which doesn't exist on
this kernel at all), diagnosed directly against `/proc/self/status` and
confirmed fixed by a later rebuild: `unshare --user --map-root-user` and
`runsc --rootless --platform systrap do true` both now succeed in this
devcontainer. Second — and this is what actually let a full `up` at
`isolation = "hardened"` complete — the Landlock profile this module
used to apply to itself before exec'ing into `runsc run`, as defense in
depth additive to gVisor's own Sentry confinement, was **removed**. It
turned out to make `--rootless` bootstrap fail unconditionally on every
host, not just this one: `runsc run`'s own chroot setup issues a
`mount()` call to change mount propagation, and that call returns
`EPERM` under *any* active Landlock ruleset regardless of what it
grants — confirmed by elimination (a ruleset granting `/` full
read-write still failed identically), and Landlock cannot mediate
`mount()` in any current ABI, so no grant could have fixed it. This had
never been exercised against a real unprivileged user namespace before
today; see `src/gvisor/runner.rs`'s module doc and
`openspec/changes/add-flox-services/tasks.md` task 6.5 for the full
evidence trail.

With both fixes in, and two more real bugs caught by the same live run
and fixed alongside them — `oci_spec::build`'s bundle never pre-created
each mount's destination directory inside `rootfs/` (gVisor's gofer
requires one to exist before it will bind onto it), and `root.path` was
a relative `"rootfs"` where gVisor's own symlink-escape guard requires
an absolute path — **a full `up` at `isolation = "hardened"` now
completes end to end**: `exec` and the SSH round trip both work (a third
bug, `runsc_command::exec_args` inserting a `--` separator `runsc exec`
doesn't expect or want, was found and fixed by this same run), and a
project declaring `[services]` gets a real `process-compose` running
inside the sandbox via `runsc exec`, with `ps`/`status`/`logs` showing
it and `down` reaping it cleanly. Every one of this tier's claims that
was previously "implemented but unverified" is now verified live, not
just reasoned about.

**`own-policy-baseline` is implemented.** Every profile devcroft compiled
used to carry 240 rules across 18 backend policy groups that `policy
--render` could not show — a typical sandbox rendered 8 rules and shipped
248. The unrendered majority came from nono injecting its full group set
into any profile, `extends: "default"` or not (confirmed with `nono
profile diff`: `extends` contributes exactly one setting, `signal_mode`).
Fixed at the root: the compiled profile now names, via `groups.exclude`,
every group it declines — `system_read_linux_core`/`system_read_macos`
(broad host `/usr/bin`, `/lib`, `/usr/share` access that contradicted
devcroft's own closure-tier thesis) and the inert `dangerous_commands*`
blocklist (verified live that `rm`/`cp` both succeed under it — `wrap`
has no resident supervisor to enforce a command blocklist, so emitting it
would claim a protection that isn't real). `signal_mode` is now set
explicitly rather than inherited. What still reaches the backend outside
devcroft's own rules — the eight required deny groups plus five narrow
optional ones (`/tmp`, `/dev` writes, a handful of `~/.local`/Homebrew
paths) this change deliberately leaves alone — is rendered too, sourced
live from `nono profile groups <name> --json` and attributed to
`backend:<group>` rather than devcroft's own `baseline`, so `policy
--render` now accounts for literally everything reaching the backend, a
claim verified by a test that resolves a real compiled profile through
nono and asserts nothing comes back unaccounted for.

The result is real, not cosmetic: `/usr/bin/gcc` and `/bin/ls` are now
denied inside every process-tier sandbox, verified live against
`samples/flox-clap-sample` (a full `cargo build` still succeeds, entirely
from the flox closure) and `samples/nix-go-sample` (`go build` too, once
`/tmp` — needed for Go's build scratch dir — was added to the sample's
own manifest, the same declaration any project needs now that the
baseline no longer grants it implicitly). Two independent, pre-existing
bugs were found and fixed along the way, both host-toolchain-passthrough
masking the same class of gap this change targets: `devcroft shell`'s and
the SSH server's `$SHELL`-then-fallback logic used to fall back to an
absolute `/bin/sh`, a host path no provider closure can ever satisfy —
now a bare `sh`, resolved by `PATH` inside the sandbox like every other
command, so a project that installs a shell into its closure gets a
working `devcroft shell`. And a generated `process-compose` services
config relied on its own undeclared `/usr/bin/bash` default, fixed by
naming `sh` explicitly (`shell_command` in the generated config) for the
same reason. `doctor`'s backend check now also exercises the actual
interface — schema validation and a live check that `groups.exclude`
still resolves the way the compiled policy assumes — rather than asserting
a version number alone, and the tested range widened to `>=0.71.0,
<0.75.0`, verified against both ends live.

**`use-nono-library` is implemented.** The process tier no longer execs a
`nono` binary at all — `nono` moved from a runtime `PATH` dependency to a
linked library, and the keeper applies the compiled policy to *itself*
directly (`nono::Sandbox::apply_auto`) right after inheriting its
listener fds, closing the fd-passing hop through a foreign process the
architecture's own listener-before-restriction invariant always described
as temporary. `nono` is no longer required on `PATH` for `up`/`exec`/
`down` to work; `doctor`'s backend check now reports kernel/platform
support (`Sandbox::support_info()`) instead of a binary version. Verified
live: a full `cargo build` under `flox-clap-sample`, with the built
binary running, `/usr/bin/gcc` and `~/.ssh` denied throughout, and no
`nono` process anywhere in the sandbox's process tree.

This is a real, security-relevant scope narrowing, not a side effect:
own-policy-baseline's rendering of nono-cli's ~100-path group catalog
(browser cookies, keychains, shell history, dotfiles beyond devcroft's
own baseline) is gone along with it — that catalog is a pure `nono-cli`
concept, invisible to the raw library. The process tier's credential/
privacy protection is devcroft's own `SENSITIVE_PATHS` (`~/.ssh`,
`~/.aws`, `~/.config/gcloud`, `~/.kube`) and `DEVCROFT_DATA_DIR`, exactly
as before, and always the load-bearing part — nono-cli's broader catalog
targets a different threat model (wrapping an arbitrary, possibly
untrusted AI agent with broad host access) than devcroft's (a project's
own code, running against a curated provider closure). Confirmed with the
project owner rather than assumed; see `openspec/changes/use-nono-library/design.md`
Decision 5 for the full reasoning.

`network.allow` (domain-level filtering) is unaffected by this change in
the sense that matters: it was already non-functional under devcroft's
`nono wrap`-based invocation (`wrap` has no resident supervisor, and
domain filtering needs one — verified live that a `curl` to an *allowed*
domain got the identical kernel-level denial as an unrelated one), and
still compiles to a plain network block under the library. Fixing it for
real is unrelated future work, not a regression this change introduces.

**Service reporting was rebuilt after a review found it silent in four
different ways.** All four shared one shape: a service problem that
showed up as *nothing at all*. `status` learned service state only by
asking `process-compose`, so anything the supervisor could not answer for
vanished rather than being reported — against the `services` spec's own
"SHALL NOT be omitted from service listings".

- **A dead supervisor looked like a healthy sandbox.** Three declared
  services plus a `process-compose` that died at startup produced output
  byte-identical to a project declaring no services; the only trace was a
  line in the keeper log. `up` now records the declared service names,
  and `status`/`ps` reconcile the live answer against them, so an
  unreachable supervisor is named and its services still listed.
- **Two of the four service states were wrong, measured live against
  process-compose 1.120.0 rather than assumed.** A service still waiting
  on a `depends_on` gate reports `is_running: false, exit_code: 0` — read
  by exit code alone, `status` called it "exited", so a service that
  hadn't started yet looked like one that had already finished. A service
  *skipped* because its dependency failed reports `exit_code: 1` that no
  process ever produced, rendering as "failed (exit 1)", an invented
  failure. Both now report as themselves. `exit_code` remains
  authoritative for services that actually ran, because a real crash
  reports `status: "Completed"` exactly like a clean exit does.
- **Service artifacts were keyed on the project root alone.** Two
  sandboxes with different names sharing one root overwrote each other's
  generated config, raced for one supervisor socket, and each reported
  the other's services. They now live in `.devcroft/<sandbox-name>/`, and
  `rm` cleans up the directory it created. Since the path grew a level,
  `up` also checks it against the OS socket-path limit and fails at layer
  `config` rather than letting the supervisor fail to bind for an
  unstated reason.
- **`.devcroft/` was gitignored nowhere**, so devcroft's own generated
  files left `git status` dirty in every worktree — worst in exactly the
  fan-out flow it targets. `init` now adds the entry.

One hardening fix came with them: `status`/`ps` read a socket the
*sandbox* controls, which is accident protection at the process tier but
a real trust inversion at `hardened`, where `--host-uds=create` exists
precisely to let the host reach inward. That read now verifies the path
is a socket owned by the invoking user, caps the response size, and
bounds the whole exchange rather than only each individual read.

**`add-devbox-provider` is implemented.** devbox is a third closure-tier
`env.provider`, resolved by capturing `devbox shellenv --pure` (never
`devbox run`, which — measured, not assumed — runs a project's
`shell.init_hook`; `shellenv` never does, in any variant) and reusing the
same fixed-baseline diff, store-grant, and staleness machinery flox and
nix already share. This is the second provider proposed purely to
confirm the `Provider` trait generalizes to a substrate the first two
don't share (devbox has its own resolver and its own lockfile format, no
flake underneath), and it does: only `src/provider/mod.rs` (dispatch
arms) and `src/provider/validate.rs` (one name moved lists) changed
shape beyond the new module. See
[samples/devbox-citytime-sample](samples/devbox-citytime-sample/) for a
working example.

Two corrections were found live while implementing, not while designing —
both narrowed what the change originally assumed devbox needed:

- **The lockfile precondition checks key presence, not per-system
  coverage.** A draft precondition required a declared package's
  `devbox.lock` entry to cover the system `up` runs on, reasoning that an
  entry resolved only for another platform leaves the current one
  unresolved. Measured against a real capture: it doesn't — devbox
  resolves any system from the entry's *pinned commit reference*, which
  is system-independent, without touching the lockfile. What actually
  contacts a package index and rewrites the lockfile — confirmed
  directly, `nixpkgs-unstable` fetched from `cache.nixos.org`, lockfile
  mutated on disk — is a declared package with **no key at all** in
  `devbox.lock`. The precondition checks exactly that.
- **Store grants need no profile-symlink resolution.** A devbox project's
  declared packages reach `PATH` through a `.devbox/nix/profile/default`
  symlink chain rather than as bare store paths, which looked like it
  would require deriving grants by resolving that chain instead of
  reusing the other two providers' scrape-`PATH`-for-`/nix/store`
  mechanism. It doesn't: that mechanism already returns only the coarse
  `/nix/store` root, never an enumerated path, and devbox's own stdenv
  wrapper puts real `/nix/store/...` entries on `PATH` regardless of
  declared packages — so the existing mechanism, reused completely
  unchanged, already grants everything the symlink resolves to. Verified
  with a package outside devbox's stdenv (ripgrep), so the claim is
  falsifiable rather than assumed.

Both are recorded in `openspec/changes/add-devbox-provider/design.md`
decisions 1a and 1b, corrected in place rather than left standing next to
their own contradiction.

**Then an adversarial review of the shipped result found the precondition
did not deliver the rule it was written for**, and that is worth stating
plainly because the change had already been committed as complete.
`up` rewrote the user's `devbox.lock` during provisioning — precisely
what the `env-provider` spec says resolution SHALL NOT do. A project
whose every *declared* package was locked still slipped through, because
`devbox.lock` also carries devbox's own base nixpkgs entry, which no
per-package check can see; `up` resolved that entry against the floating
`nixpkgs-unstable` branch and wrote it to disk. Now enforced by comparing
the lockfile's bytes across capture and restoring + failing on any
change — a byte comparison rather than a larger precondition, since the
base entry's key is not a constant (a project pinning `nixpkgs.commit`
locks under a different one) and predicting it would mean reimplementing
devbox's resolution rules.

That carried a second correction with it: **"declares no packages" does
not mean "nothing to resolve"**. A zero-package devbox project still gets
its stdenv from that same unpinned base, so it is reproducible only once
`devbox install` has written a lockfile. The spec scenario asserting such
a project needs none, `init`'s matching advice, and three tests were all
wrong in the same way, and are corrected together. Two of this change's
own tests had also been passing for the wrong reason, because `devbox
add` — unlike `devbox install` — does not write a complete lockfile.

## Limitations

devcroft's default (and only fully implemented) tier, `process`, is
Landlock or Seatbelt applied to a process tree. **This is accident
protection, not a security boundary** — the full host kernel syscall
surface stays reachable from inside, so a kernel bug is an escape. A real
boundary is the `hardened` tier (gVisor via `add-gvisor-backend`, or
LiteBox; see
[openspec/changes/add-hardened-tier/](openspec/changes/add-hardened-tier/)),
implemented and now verified end to end against a live sandbox — see the
Status section above for what that verification covered. Every isolation
claim in this README and in `devcroft`'s own output is scoped to
`process` unless said otherwise.

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

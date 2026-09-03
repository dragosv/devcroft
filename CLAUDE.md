# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository state

The MVP (`add-mvp-core`) is implemented and in its final stretch — **24/25
tasks**, the last one being the publish itself. `src/` has real modules for `config`, `policy`, `provider`,
`keeper`, `lifecycle`, `ssh`, `services`, plus the `devcroft` and `spike`
binaries under `src/bin/`, backed by an integration `tests/` suite. Stack
is Rust stable, edition 2024. `samples/` holds standalone example projects
covering the three closure-tier providers —
`flox-clap-sample`, `flox-rustup-sample`, `nix-flake-sample`, and
`devbox-citytime-sample` are Rust projects with their own `Cargo.toml`
(each has an explicit `[workspace]` table so they don't get pulled into
this crate's workspace); `nix-go-sample` (Go),
`nix-probe-sample` (Go), `kotlin-ktor-sample`
(Kotlin/Gradle — was `gvisor-kotlin-sample`, renamed by
`remove-gvisor-backend`, which also dropped the `isolation = "hardened"`
key from its manifest that would otherwise now fail to parse), and
`flox-services-sample` (no application code at all)
are non-Rust, so no workspace exclusion applies to them — see each
sample's own `README.md` for what it demonstrates. `flox-services-sample`
shows `network.ports` and supervised `[services]` both working — devcroft
generates its own process-compose config and the keeper owns the
services' lifetime. It is also the regression case for the shell
invariant below, because its manifest declares no shell, which is what
every real flox manifest looks like.
`nix-probe-sample` is the runnable form of the README's boundary probe,
and is where that front-page output is measured rather than asserted. It
is also the sample that established three nix-provider facts the other
two nix samples do not exercise: a `shellHook` export never arrives
(`print-dev-env --json` treats the hook as inert data, so redirects must
be `mkShell` *attributes*), nix reports a `TMPDIR` that no longer exists
at session time and cannot be overridden by an attribute (`GOTMPDIR` is
the lever), and `devcroft.toml`'s `[env.vars]` parses and validates but
nothing consumes it — a silent no-op. Its deletion probe targets a file the
*user* creates by hand (`touch ~/devcroft.tmp`), never one the program
makes: a demonstration of a boundary must not be catastrophic when the
boundary is absent, and a program that tried to create the file itself
measured nothing, since creating it in `$HOME` is refused by the same
boundary the deletion is meant to test — the removal then returned
`ENOENT`, which is not evidence of a refusal.

`devbox-citytime-sample` documents a real constraint rather than a gap:
devcroft's devbox provider never runs `shell.init_hook` (by design — see
the two-phase execution invariant below), so unlike the flox and nix
samples it has no host-side hook to fetch crates.io dependencies in, and
depends on nothing beyond `std` as a result.

Remaining work (see `openspec/changes/add-mvp-core/tasks.md`): task 7.5,
and only its last step — running `cargo publish` and reserving the npm
name, both of which need the maintainer's own accounts. Everything
checkable is checked: `cargo package` verifies, clippy and rustdoc are
warning-free, and the name is free on both registries (2026-08-31).

**The first release is `0.0.1`, not `0.1.0`, deliberately.** `0.0.z` is
the only range cargo treats as incompatible with itself, which is the only
numbering consistent with what `src/lib.rs` already states — internals
published so `tests/` can drive them, no stability offered — and it avoids
putting a confident number on a boundary `tests/unix_socket_not_mediated.rs`
still shows open. `docs/roadmap.md` holds the rule and says when `0.1.0`
gets cut (when `add-mount-isolation` lands). Task 6.5, the editor matrix,
was closed on its deliverable rather than on Zed working: the matrix
documents what each editor needs, negatives included
(`docs/ssh-validation.md`), and a release held on a third party's
unattributed bug is held indefinitely.

For current status, see the README's Status section — kept short on
purpose; full detail behind each published gap is in
`docs/known-gaps.md`. For the blow-by-blow of what was built and **what
turned out to be wrong along the way**, see `docs/implementation-log.md`.
For the longer competitive reasoning, see `docs/comparison.md`. All three
used to live in the README (2026-08-30: it had a 100+-line comparison
essay and a duplicated, partly-stale gap list; before that,
`docs/implementation-log.md` was extracted when the README hit 376
lines). The README is now restyled to a short, ecosystem-standard shape —
overview, install, usage, features, platform support, status, limitations,
docs, license — matching the `nono` library's own README. Do not
duplicate any of these here.

## Working commands

```sh
cargo build                 # build the devcroft binary + spike
cargo test                  # integration tests; self-skip on a host that cannot run them
cargo clippy                # lint; currently clean
cargo fmt                   # format
cargo doc --no-deps         # rustdoc; currently zero warnings — keep it there,
                            # docs.rs renders these publicly
```

**A green `cargo test` is not the same as a run that tested anything.**
Every e2e test skips itself on a host that cannot support it, and a skip
looks exactly like a pass in cargo's default output. Guard on the
*capability*, never on the binary — `flox --version`, `flox init`,
`nix flake --help` and `devbox version` all succeed with an unreachable
Nix store, and every test that then tried to build an environment failed
in a way that read as a devcroft regression. `provider::host_can_build_nix_closures()`
is the shared probe (alongside `policy::backend_supported()`); it connects
to the daemon socket, and treats a *missing* socket as usable, since a
single-user store has none. To see what a run actually skipped:

```sh
cargo test -- --nocapture 2>&1 | grep skipping
```

On a devcontainer whose `nix-daemon` is not running that is ~80 tests,
which is most of the interesting ones.

**After any `Cargo.lock` change**, regenerate the dependency attribution:

```sh
python3 scripts/gen-third-party-licenses.py   # rewrites THIRD-PARTY-LICENSES.md
```

devcroft is **Apache-2.0** (`LICENSE-APACHE` + `NOTICE`), matching nono
and the sigstore crates it links rather than the Rust-conventional dual
`MIT OR Apache-2.0` — a dual license would let a user take MIT and no
patent grant while changing none of their real obligations, since nono,
russh and every `sigstore-*` are Apache-2.0-only and linked regardless.
189 of the 335 shipped dependencies are Apache-2.0, whose §4(a) requires
recipients get a copy of the License.
The generator's non-obvious part: `nono`, `russh` and every `sigstore-*`
declare Apache-2.0 but vendor no license file, so it substitutes the
canonical text for those — exact, not approximate, because the Apache-2.0
text has no per-holder copyright line (unlike MIT, which is why MIT is
never substituted).

**Packaging is an anchored `include` allowlist** in `Cargo.toml`. Without
the leading `/` on each pattern the globs match at any depth and sweep in
`samples/`, flox store symlinks, and a vendored Go module cache. If you
add a file the published crate needs, add it there — the default would
otherwise ship `openspec/` (131 files) and `.claude/` permanently.
The one negation, `!/src/bin/spike.rs`, is load-bearing rather than tidy:
`src/bin/` targets are auto-discovered, so shipping that file makes
`cargo install devcroft` drop a second, generically-named `spike` binary
on the user's PATH. Check what the package *installs*, not just how many
files it holds — the first packaging audit got the count right and missed
this.

This project is also spec-driven via the [OpenSpec](https://github.com/Fission-AI/OpenSpec) CLI:

```sh
openspec list                                  # active changes + task progress
openspec validate --all                        # validate every change
openspec validate <change> --type change       # details for one change
openspec status --change <change> --json       # artifact state, paths, what's next
openspec instructions <artifact> --change <c> --json   # how to write an artifact
```

`openspec validate --all` currently reports **18 passed, 0 failed**.
`add-nix-provider` (nix flakes as a second closure-tier environment
provider, alongside flox), `add-hardened-tier` (whose tier dispatch
`remove-gvisor-backend` deleted; its backend-generic `SessionBackend`
seam is deliberately kept), `own-policy-baseline`,
`use-nono-library`, and `fix-provisioning-hooks` are all fully
implemented, tasks.md and all
— see `docs/implementation-log.md` for what each one found along the way.
`add-devbox-provider` (a third
closure-tier environment provider) is implemented too, with one task
(3.6, a services-loud-failure test) left unchecked. **Its stated blocking
reason is now stale**: it says the mechanism exists for no provider, but
`add-flox-services` task 2.4 built it — `prepare_services` calls
`ensure_no_services_declared_for_another_provider`, covered by
`services_declared_for_another_provider_fail_rather_than_being_ignored`.
Re-derive before treating 3.6 as blocked. `remove-gvisor-backend` is now **complete**: its last task waited on
`add-backend-capabilities`, which did not exist and has since been written
(the task was to *rewrite* a change that was never authored, so writing it
was the resolution). Those plus `add-mvp-core` are the
changes actually implemented; run `openspec list` for the rest, which are
in flight or not started.

**Ideas taken from other projects are recorded in `docs/prior-art.md`** —
what was taken and from where, so an idea whose origin is lost does not
get re-litigated. devcroft takes *techniques*, never tools: two standing
requirements make that a rule (the keeper "SHALL NOT be executed as a
child of a separate sandboxing binary"; "the process tier requires no
external backend binary"), so bubblewrap and `sandlock` are references,
not candidates.

**Sequencing for what is left is in `docs/roadmap.md`** — what 0.2 through
1.0 each have to be true for, and why in that order. Two entries there
matter before touching the relevant change:

- **`add-mount-isolation` is 0.2, ahead of everything else**, because it
  makes a *shipped* claim true rather than adding a new one. Landlock does
  not mediate AF_UNIX, so every sandbox today reaches any world-accessible
  unix socket — the nix daemon's included, with `/nix` ungranted.
  `tests/unix_socket_not_mediated.rs` asserts that gap and passes *because
  it is open*; closing it must correct that test and three documents
  together. The fix is a mount namespace, not seccomp (measured).
- **Fleet's D9 gate is suspended, not struck.** It declared "no proxy work
  starts until the seccomp handoff resolves", reasoning that a userspace
  network helper makes proxy variables cooperative. The shipped design has
  no such helper — loopback-only namespace, egress via a unix-socket relay
  — so a workload ignoring `HTTPS_PROXY` is refused by Landlock's
  `NetPort` and by having no route out. Re-derive before building either
  way; if it holds, fleet loses its hardest phase-0 item.

`own-policy-baseline` and `use-nono-library` came out of measuring what
devcroft's compiled profile actually contains: 240 rules it ships and
cannot render. `extends: "default"` is *not* where they come from — nono
injects its group set into every profile, and `extends` contributes only
`signal_mode: Isolated`. The lever is `groups.exclude`, and the gate for
the whole change is whether a build survives excluding
`system_read_linux_core`. Measured: it does — a full Rust build from a
flox closure needs the project root, `/tmp`, `/nix/store` and 19
`/dev`+`/proc` entries, with `/usr/bin/gcc` and `/bin/ls` denied.

That result is what removed `add-mise-provider`. An artifact-tier
provider is host-linked by definition, so it can no longer inherit
library access from the baseline; it must declare those grants itself,
rendered with a `provider:<name>` origin. mise still passes the six
criteria — see `docs/decisions.md` §1, which now carries the constraint.

`use-nono-library` depended on `own-policy-baseline` and is now
implemented: the process tier links nono as a library and self-restricts
via `nono::Sandbox::apply_auto`, rather than exec'ing `nono wrap`. Its
one recorded objection — the 141-crate trust/verification dependency
tail — was accepted by the project owner (proposal.md, design.md
Decision 4). The single open task is 6.4, filing the upstream ask that
nono gate its trust module behind a Cargo feature; it is deliberately
left for the owner to send, since filing an issue on a third-party repo
is an external action an agent should not take unprompted.

**`add-egress-proxy`'s design.md E6 proposes adopting a second crate from
the same project, `nono-proxy` 0.74.0**, and the same objection applies
again with a measured number: **116 additional crates**, the same order as
the trust tail above — **not yet taken**, and not urgent, for a reason
worth recording precisely because it changes the calculus stated when this
was first written. Reading that crate's config surfaced a real defect in
devcroft's own proxy — no authentication on a loopback listener, which
makes it an open relay lending its sandbox's allowlist to any local
process — and the defect was **fixed directly in devcroft's own proxy**
(task group 4a: a per-session token as proxy-URL userinfo, checked before
the allowlist decision), rather than by importing the crate. Adopting
`nono-proxy` remains open as a *separate* decision for what else it
brings — credential brokering, approval hooks, audit integrity — and
should be judged on that trade alone, not as the fix for a gap that no
longer exists.

Three of that crate's capabilities, if it is ever adopted, are **off by
decision, not by omission** — TLS interception (an explicit non-goal),
SPIFFE, and AWS routing. Enabling any of them is a change to what devcroft
claims, not a configuration tweak.

Skills `/opsx:propose`, `/opsx:update`, `/opsx:apply`, `/opsx:archive`,
`/opsx:sync`, and `/opsx:explore` drive the workflow.

## OpenSpec layout rules

```
openspec/
  config.yaml                    # schema + `context:` (project context fed to agents)
  changes/<change-id>/
    .openspec.yaml               # REQUIRED marker; without it the change is invisible
    proposal.md  design.md  tasks.md
    specs/<capability>/spec.md   # delta specs
  changes/archive/
  specs/<capability>/spec.md     # MAIN specs — empty until the first archive/sync
```

Three things that are easy to get wrong:

- `openspec/specs/` holds **main** (synced) capability specs, not change
  directories. Change dirs go under `openspec/changes/`.
- Every change dir needs `.openspec.yaml` (`schema: spec-driven`) or the CLI
  will not discover it.
- Project context lives in `openspec/config.yaml` under `context:`.
  `openspec/project.md` is **legacy** — the CLI's `legacy-cleanup` module
  flags it for manual migration. Do not recreate it.

Delta specs must use `## ADDED|MODIFIED|REMOVED|RENAMED Requirements`, then
`### Requirement:`, and every requirement needs at least one
`#### Scenario:` block or validation fails.

## Architecture invariants

These are load-bearing and non-obvious; violating one is a design error, not
a style issue. Rationale is in `openspec/changes/add-mvp-core/design.md`.

**Listener-before-restriction ordering.** Landlock and Seatbelt apply to a
process tree and are inherited by children; there is no API to join an
existing sandbox from outside. So `up` must create the unix listener sockets
*first*, then spawn the keeper with the fds inherited, then have the keeper
apply the compiled profile **to itself**. The sockets stay reachable from
outside only because they predate the restriction. This fd-passing trick is
the load-bearing risk of the whole design and is retired first in the task
order.

**Two-phase execution, fixed and non-negotiable.** Provider provisioning
(package materialization, environment capture) runs host-side at `up`,
*before* restrictions, using the host's own network. Everything after
restriction — sessions and devcroft's own manifest hooks — runs inside the
boundary. The manifest's `[network]` and `[filesystem]` sections govern
**runtime only, never provisioning**. devcroft's `[hooks]` are project code
and never get provisioning privileges; a hook that needs the network needs
an allowlist entry.

The provisioning phase is trusted because it runs pinned tooling from a
lockfile — but **that is a goal, not a guarantee, and the gap is
provider-dependent** (`fix-provisioning-hooks`). A provider's *own*
activation hook is project code too, and some providers cannot be asked
for an environment without running it. Measured:

- **nix**: fixed. `print-dev-env --json` returns the build environment as
  data, `shellHook` included as an inert string. Never evaluated. Note
  that plain `print-dev-env` is *not* a fix — the script it emits ends
  with `eval "${shellHook:-}"`.
- **devbox**: fixed, and implemented (`add-devbox-provider`). `shellenv
  --pure` does not run `init_hook`, in any variant; `devbox run` does.
  Uses the former.
- **flox**: **fixed by devcroft, not by a flox flag — and implemented.**
  No `flox activate` mode suppresses `[hook].on-activate`: not
  `--mode run`, `--mode dev`, nor `--no-start-services`. devcroft therefore
  materializes from a **derived, hook-free copy** of the environment it
  owns, and runs the hook *inside* the sandbox
  (`flox::derive_hook_free_env`, `hooks::run_activation_script`;
  `sandbox-provisioning` P2d). Measured: stripping `[hook]` yields a
  byte-identical locked package set and an identical store path, because a
  hook is not a package input — a genuine split, not a different
  environment. Asserted in `tests/flox_derived_env.rs`.

  Consequence worth knowing before it surprises someone: a flox hook now
  runs under the *manifest's own policy*, so one reaching for host tooling
  is denied where it previously succeeded. That is `own-policy-baseline`
  working as designed — a hook's commands must come from the closure —
  but it is a real behaviour change for hooks that assumed host access.

  **That answer is context-dependent, and the contexts are about to
  diverge — do not read the two as contradictory.** Warning is right
  *today*, where provisioning is unconfined either way, so refusing would
  block a user from something their own shell does identically. Under
  `sandbox-provisioning` the promise changes to "activation is confined",
  and flox-with-a-hook cannot keep it: materialization needs `nix-daemon`
  authority, the hook is project code, and flox cannot separate them — so
  there it **fails closed at layer `provider`** rather than silently
  handing project shell a host-global capability (that change's P2b/P2c).
  Same measured fact, different promise, opposite correct behaviour. The
  upstream request that would collapse the two back together is drafted
  at `docs/flox-confined-activation-issue.md`.

The rule for any new provider: prefer the entry point that hands back an
environment over the one that runs a command inside it, since the latter
runs hooks in every provider measured so far. Where no such entry point
exists, report it — never let it pass silently.

**Environment resolves once, at `up`.** Provider activation runs once and is
captured as an env diff, then injected into the keeper. Sessions inherit it.
Changing the underlying flox manifest requires `up --recreate`. Do not
propose per-session activation — it would force the profile to grant flox
internals forever.

**Provider resolution must not widen the policy.** If activation would need
write access outside the project root, `up` fails naming the path rather than
silently granting it.

**Policy is deterministic and inspectable.** Manifest + provider grants +
baseline compile byte-identically into `<state>/<name>/profile.json`. Every
rule carries an origin: `manifest:<key>`, `provider:<name>`, or `baseline`.
Nothing goes to the backend that cannot be shown via `policy --render`.
Baseline denials always win, including devcroft's own data dir.

**SSH is reachable only on a unix socket, never TCP.** The keeper embeds
russh listening on a 0600 socket in a 0700 state dir. The filesystem
permissions are the real access boundary — not the process's location — and
SSH exists for editor protocol compatibility, never network security. Clients
reach it via `ProxyCommand devcroft proxy %n`.

This used to be stated as tier-dependent, because the removed hardened tier
ran the SSH/control server host-side and dispatched sessions through the
backend's own exec-into primitive. The underlying invariant was the same in
both cases, which is why removing the tier did not change it — worth knowing
if a second backend ever arrives, since it would face the same choice.

**Degraded capabilities are surfaced, never silent.** If the host cannot
enforce a requested aspect (e.g. domain allowlists on macOS Seatbelt), `up`
prints exactly one warning naming the aspect, the reason, and the fallback.
Never drop a capability quietly.

**devcroft's shell is devcroft's dependency, not the project's.** SSH
login sessions, `devcroft shell`'s fallback, and the command
process-compose runs each service through all need a POSIX shell. No real
flox/nix/devbox manifest declares one, because under a plain
`flox activate` the host's `PATH` still resolves `sh` —
`own-policy-baseline` removed host toolchain access without replacing it,
so all three silently began resolving to a host path the policy denies.
`src/shell.rs` resolves an absolute shell at `up`, from the closure: the
resolved `PATH` **only where the hit is inside `/nix/store`** (that guard
is the whole correctness — a provider's `PATH` ends in the host's own
directories, and the first version of this picked `/usr/bin/dash`), else
a `bin/sh` from the closure's requisites. The path is recorded in
`Meta.shell`, handed to the keeper as `DEVCROFT_SHELL`, and its store
root is folded into the provider grants **in `up()`**, so `Meta` and the
compiled profile cannot disagree — `policy --render` renders from `Meta`,
and a rule the backend gets that `--render` cannot show breaks the policy
invariant. Never reintroduce a bare `"sh"` at any of the three call
sites, and never require the project to declare one: that would fail the
first `up` of every existing project for a dependency devcroft
introduced.

**Error contract.** Every error names its layer — `config` | `provider` |
`backend` | `keeper` | `ssh`. Exit codes are stable: 0 success, 1 runtime,
2 usage/config, 3 environment/provider, 4 backend, 5 keeper/connection.
Never prompt when stdout is not a tty; `rm` and `up --recreate` need `--yes`
non-interactively.

## Capability map

Read the relevant spec before changing behavior that touches it. All under
`openspec/changes/add-mvp-core/specs/`:

| Capability | Owns |
|---|---|
| `config` | `devcroft.toml` schema, discovery walk, validation |
| `policy` | Manifest → backend profile compilation, `policy --render`, `why` |
| `env-provider` | Provider trait, flox resolution, staleness, provider rejection |
| `lifecycle` | Keeper process, `up`/`down`/`rm`, `status`/`logs`/`ps`, hooks |
| `exec` | `exec` and `shell` sessions, pty, signals, exit codes |
| `ssh` | Embedded server, `proxy`, `ssh-config`, key management |
| `cli` | Command surface, name resolution, `init`, `doctor`, error contract |

MVP command surface is closed: `init`, `up`, `down`, `rm`, `status`, `logs`,
`ps`, `shell`, `exec`, `ssh`, `proxy`, `ssh-config`, `policy`, `why`,
`doctor`. Anything else is post-MVP.

`help`/`--help`/`-h` and `--version`/`-V` are not additions to that
surface — they describe it. `src/bin/devcroft.rs`'s `USAGE` is the only
place a user of the *published* binary can discover what exists, so a new
command must be listed there;
`tests/cli_help_and_version.rs` fails if one is not.

## Before proposing a feature

`docs/decisions.md` is the reference for every "why doesn't devcroft support
X" question, and it is written to be falsifiable: each rejection names the
specific property that fails, not a preference. Check it first — the answer
is often already there, in one of three categories: rejected by design,
covered differently, or a known gap.

Environment providers are judged against a six-criterion test (§1):
declarative manifest, restorable lockfile, immutable-capable shared store,
capturable activation, completeness, verifiable preconditions. A provider
that covers language runtimes but leaves the C toolchain to the host has
smuggled `host` passthrough back in under another name.

If a rejection's stated reason stops being true — upstream changed, new
mechanism exists — the decision should be **revisited, not defended**.

## Framing rules

- There is **one** isolation tier (Landlock/Seatbelt), and it is accident
  protection, not a security boundary. The gVisor-backed `hardened` tier was
  built and removed (`remove-gvisor-backend`; code recoverable at the tag
  `gvisor-backend-last`) — Landlock cannot mediate `mount()`, which `runsc`
  requires, so the two could not be stacked at all. The supported answer for a
  stronger boundary is running devcroft inside a VM, as macOS already does.
  Never make blanket security claims; see `docs/threat-model.md` for which use
  case is backed and which is not.
- Guarantee tiers (`closure` vs `artifact`) are always user-visible. Do not
  market two different guarantees under one word.
- There is no non-reproducible mode. `host` and `none` providers are out of
  scope by design; the answer for a project without an environment is
  `flox init`, not a degraded fallback. Rejection messages must distinguish
  "not yet supported" (devenv, mise, pixi, hermit) from "out of
  scope by design" (`host`, `none`) and from "fails the qualification
  test" (version managers). Nix flakes and devbox are implemented, not
  pending — devbox is the third closure-tier `env.provider`
  (`add-devbox-provider`), confirming the `Provider` trait generalizes to
  a substrate flox and nix don't share.
- Known limitations are published, not hidden: no inter-sandbox process
  visibility separation in MVP, cooperative/platform-dependent network
  filtering, no cgroup resource limits.

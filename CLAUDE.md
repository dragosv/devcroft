# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository state

The MVP (`add-mvp-core`) is implemented and in its final stretch — **23/25
tasks**. `src/` has real modules for `config`, `policy`, `provider`,
`keeper`, `lifecycle`, `ssh`, `services`, plus the `devcroft` and `spike`
binaries under `src/bin/`, backed by an integration `tests/` suite. Stack
is Rust stable, edition 2024. `samples/` holds standalone example projects
covering the three closure-tier providers —
`flox-clap-sample`, `flox-rustup-sample`, `nix-flake-sample`, and
`devbox-citytime-sample` are Rust projects with their own `Cargo.toml`
(each has an explicit `[workspace]` table so they don't get pulled into
this crate's workspace); `nix-go-sample` (Go), `kotlin-ktor-sample`
(Kotlin/Gradle — was `gvisor-kotlin-sample`, renamed by
`remove-gvisor-backend`, which also dropped the `isolation = "hardened"`
key from its manifest that would otherwise now fail to parse), and
`flox-services-sample` (no application code at all)
are non-Rust, so no workspace exclusion applies to them — see each
sample's own `README.md` for what it demonstrates. `flox-services-sample`
is the one that documents an *unfinished* capability on purpose: it shows
`network.ports` working and `[services]` being parsed, and demonstrates
that devcroft does not yet supervise those services.
`devbox-citytime-sample` documents a real constraint rather than a gap:
devcroft's devbox provider never runs `shell.init_hook` (by design — see
the two-phase execution invariant below), so unlike the flox and nix
samples it has no host-side hook to fetch crates.io dependencies in, and
depends on nothing beyond `std` as a result.

Remaining work (see `openspec/changes/add-mvp-core/tasks.md`): task 6.5
(cross-editor SSH validation matrix — OpenSSH, rsync, VS Code Remote-SSH,
and Cursor are all validated against a live sandbox; only Zed remains, no
CLI to drive it non-interactively — see `docs/ssh-validation.md`) and task
7.5 (publish the `devcroft` crate, reserve/point the npm name).

For full status detail — what's implemented, what was fixed along the way,
which gaps are known — see the README's Status section, which is kept
current; do not duplicate it here.

## Working commands

```sh
cargo build                 # build the devcroft binary + spike
cargo test                  # integration tests; self-skip if flox/nono missing from PATH
cargo clippy                # lint; currently clean
cargo fmt                   # format
```

This project is also spec-driven via the [OpenSpec](https://github.com/Fission-AI/OpenSpec) CLI:

```sh
openspec list                                  # active changes + task progress
openspec validate --all                        # validate every change
openspec validate <change> --type change       # details for one change
openspec status --change <change> --json       # artifact state, paths, what's next
openspec instructions <artifact> --change <c> --json   # how to write an artifact
```

`openspec validate --all` currently reports **16 passed, 0 failed**.
`add-nix-provider` (nix flakes as a second closure-tier environment
provider, alongside flox), `add-hardened-tier` (whose tier dispatch
`remove-gvisor-backend` deleted; its backend-generic `SessionBackend`
seam is deliberately kept), `own-policy-baseline`,
`use-nono-library`, and `fix-provisioning-hooks` are all fully
implemented, tasks.md and all
— see the README's Status section. `add-devbox-provider` (a third
closure-tier environment provider) is implemented too, with one task
(3.6, a services-loud-failure test) deliberately left unchecked — the
mechanism it would test doesn't exist yet for *any* provider, and belongs
to whichever change finishes `add-flox-services`'s own unimplemented
"services requested from a provider that cannot supply them fail loudly"
requirement. `remove-gvisor-backend` is implemented at 16/17 for the same
kind of reason: its last task rewrites `add-backend-capabilities` for a
single backend, and that change does not exist in this repo yet — blocked
on something absent, not skipped. Those plus `add-mvp-core` are the
changes actually implemented; run `openspec list` for the rest, which are
in flight or not started.

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
- **flox**: **not fixable.** No `flox activate` mode suppresses
  `[hook].on-activate` — not `--mode run`, `--mode dev`, or
  `--no-start-services`. devcroft detects the hook and `up` prints one
  warning; refusing was rejected because `on-activate` is idiomatic flox
  and the user's own `flox activate` runs it identically.

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

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository state

The MVP (`add-mvp-core`) is implemented and in its final stretch — **23/25
tasks**. `src/` has real modules for `config`, `policy`, `provider`,
`keeper`, `lifecycle`, `ssh`, plus the `devcroft` and `spike` binaries under
`src/bin/`, backed by an integration `tests/` suite. Stack is Rust stable,
edition 2024. `samples/` holds two standalone example projects
(`flox-clap-sample`, `flox-rustup-sample`) with their own `Cargo.toml`s
(each has an explicit `[workspace]` table so they don't get pulled into
this crate's workspace) — see each sample's own `README.md` for what it
demonstrates.

Remaining work (see `openspec/changes/add-mvp-core/tasks.md`): task 6.5
(cross-editor SSH validation matrix — OpenSSH client/rsync/VS Code
Remote-SSH — partially done, see `docs/ssh-validation.md`) and task 7.5
(publish the `devcroft` crate, reserve/point the npm name).

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

`openspec validate --all` currently reports **3 passed, 0 failed**.
`add-mise-provider` and `add-hardened-tier` are still proposal-only
post-MVP sketches (not implemented, no tasks.md), but each now carries
real delta specs consistent with its proposal.md, so the validator's
"at least one delta spec per change" requirement is satisfied honestly
rather than left failing. Only `add-mvp-core` is actually implemented.

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
*before* restrictions, using the host's own network — trusted because it
executes pinned tooling from a lockfile, not project code. Everything after
restriction — sessions and hooks — runs inside the boundary. The manifest's
`[network]` and `[filesystem]` sections govern **runtime only, never
provisioning**. Hooks are project code and never get provisioning privileges;
a hook that needs the network needs an allowlist entry.

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

**SSH lives inside the boundary, on a unix socket only.** The keeper embeds
russh listening on a 0600 socket in a 0700 state dir; it MUST NOT bind TCP.
The filesystem permissions are the real access boundary — SSH is there for
editor protocol compatibility, not network security. Clients reach it via
`ProxyCommand devcroft proxy %n`.

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

- Claims about isolation are always **tier-qualified**. The default `process`
  tier (Landlock/Seatbelt) is accident protection, not a security boundary;
  a real boundary is the planned `hardened` tier. Never make blanket security
  claims.
- Guarantee tiers (`closure` vs `artifact`) are always user-visible. Do not
  market two different guarantees under one word.
- There is no non-reproducible mode. `host` and `none` providers are out of
  scope by design; the answer for a project without an environment is
  `flox init`, not a degraded fallback. Rejection messages must distinguish
  "not yet supported" (nix flakes, devbox, mise) from "out of scope by
  design".
- Known limitations are published, not hidden: no inter-sandbox process
  visibility separation in MVP, cooperative/platform-dependent network
  filtering, no cgroup resource limits.

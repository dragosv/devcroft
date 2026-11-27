# AGENTS.md

Instructions for coding agents working in this repository. `CLAUDE.md` imports
this file. Code comments that cite "CLAUDE.md's <rule>" mean the rules named in
the following sections. The reasoning and measurements behind each rule are in
[docs/invariants.md](docs/invariants.md).

devcroft is a Rust CLI (edition 2024, `rust-version` 1.95) that runs
development workloads and coding agents in sandboxes. The process backend links
the nono library (Landlock on Linux, Seatbelt on macOS) and shares the host
kernel. It protects against accidents, not hostile code.

## Before you change anything

1. Run `openspec list` and find the change that owns the behavior. Read its
   capability spec first (see [OpenSpec](#openspec)).
2. Before you propose a provider or a feature, or reopen a rejected design,
   read [docs/decisions.md](docs/decisions.md). Each rejection names the
   property that fails. If that property is no longer true, revisit the
   decision instead of defending it.
3. Before you describe the boundary, read
   [docs/threat-model.md](docs/threat-model.md). Before you claim a
   capability, check [docs/known-gaps.md](docs/known-gaps.md).

## Find the code

| Area | Start here |
|---|---|
| CLI, config, capability reporting | `src/bin/devcroft.rs` (`USAGE`), `src/config/`, `src/backend_capabilities.rs` |
| Startup, teardown, sessions | `src/lifecycle/`, `src/keeper/`, `src/exec.rs`, `src/ssh/` |
| Filesystem, network, credentials | `src/policy/`, `src/fleet/mount.rs`, `src/proxy/` |
| Toolchains, activation, services | `src/provider/`, `src/runtime/`, `src/shell.rs`, `src/services/` |
| Sandbox user | `src/identity.rs` |
| Fleet | `src/fleet/`, `tests/fleet_init.rs` |
| Test rows and examples | `tests/common/mod.rs`, `samples/`, `.github/workflows/ci.yml` |

## Commands

These match CI. [CONTRIBUTING.md](CONTRIBUTING.md) has prerequisites and the
provider rows.

```sh
cargo build --locked
cargo test --locked --test cli_help_and_version      # fast; needs no provider
cargo fmt --all -- --check
cargo clippy --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps
DEVCROFT_TEST_PROVIDER=test cargo test --locked --features test-support -- --nocapture
cargo test -- --nocapture 2>&1 | grep skipping       # list what a run skipped
openspec validate CHANGE_ID --type change --strict
python3 scripts/gen-third-party-licenses.py           # after any Cargo.lock change
cargo package --locked --allow-dirty --no-verify --list
```

## Verify what ran

- A green `cargo test` is not the same as a run that tested anything. Every
  e2e test skips itself on a host that can't run it, and a skip looks exactly
  like a pass. Grep the output for `skipping`, and report what was skipped.
- Guard a test on the capability, never on the binary. Use
  `provider::host_can_build_nix_closures()` and `policy::backend_supported()`.
  `flox --version`, `flox init`, `nix flake --help` and `devbox version` all
  succeed with an unreachable Nix store.
- For a boundary change, test a denied operation and a permitted control.
  Check Linux and macOS separately: Landlock can't subtract a protected child
  from a directory grant, and Seatbelt can.
- Report the OS, the backend, the provider rows and every material skip.
  Separate what you ran from what you inferred. Never present fixture coverage
  as real-provider or cross-platform qualification.
- Keep rustdoc at zero warnings; docs.rs renders it. After a sample whose
  closure carries the Apple SDK has been brought up, `cargo doc` also prints
  about 24 "File system loop found" warnings about `samples/*/.devenv/profile`.
  Those aren't rustdoc's, and `[workspace] exclude` doesn't suppress them.

## Invariants

Each of these is load-bearing; breaking one is a design error. The names match
the code comments.

- **Listener-before-restriction ordering.** `up` creates the Unix listeners,
  spawns the keeper with those descriptors, and the keeper restricts itself
  before it admits a session. Workloads never inherit control descriptors.
- **Two-phase execution.** Provider provisioning runs host-side at `up`,
  before restriction. Sessions, devcroft's hooks and provider activation
  scripts run inside the boundary. `[network]` and `[filesystem]` govern
  runtime only, and provisioning egress never carries into sessions. For a new
  provider, use the entry point that returns the environment as data, and
  never run a provider hook host-side. If no such entry point exists, report
  it.
- **Environment resolves once, at `up`.** Sessions inherit the captured
  environment. Don't add per-session activation.
- **Provider resolution must not widen the policy.** Fail and name the path.
  An artifact-tier provider declares its own library grants, with origin
  `provider:<name>`.
- **Policy is deterministic and inspectable.** Every rule carries an origin.
  Nothing reaches the backend that `policy --render` can't show, and baseline
  denials win. When you add an origin, add it to the table in
  [docs/invariants.md](docs/invariants.md#policy-is-deterministic-and-inspectable).
  Keep `why`, `doctor` and `status` consistent with what's enforced.
- **Scratch is scoped to the sandbox.** Use `TMPDIR`. Never share a temp or
  runtime directory between two keepers.
- **SSH is reachable only on a Unix socket**, never TCP. File permissions are
  the boundary.
- **Degraded capabilities are surfaced, never silent.** Print exactly one
  warning that names the aspect, the reason and the fallback.
- **The shell invariant: devcroft's shell is devcroft's dependency.** Resolve
  it from inside a declared grant (`src/shell.rs`). Never reintroduce a bare
  `"sh"` at any of its three call sites, and never require a project to
  declare a shell.
- **A sandbox has its own user.** On Linux it's `devcroft`, uid 1000, never 0.
  A manifest never names a host id.
- **Error contract.** Every error names its layer (`config`, `provider`,
  `backend`, `keeper` or `ssh`). Exit codes are stable: 0 success, 1 runtime,
  2 usage or config, 3 environment or provider, 4 backend, 5 keeper or
  connection. Never prompt when stdout isn't a TTY. `rm` and `up --recreate`
  need `--yes` non-interactively.
- **Network topology.**
  - The route-less namespace is the boundary. Don't add slirp4netns or pasta;
    either one reopens fleet's D9 gate.
  - Keep `nono-proxy`'s `strict_connect_auth` and `strict_filter` set.
  - Log authentication failures as `requested=`, never `host=`.
  - TLS interception, SPIFFE and AWS routing stay off.

## Rules for changes

- Fail closed when required enforcement is unavailable. Never silently drop a
  denial, broaden a grant or fall back to host tools.
- Treat repository input as untrusted. Compare path components, not string
  prefixes. Account for symlinks, aliases and check-then-use races. Check the
  effective grants after combining provider, CLI and user inputs.
- Keep credentials and private runtime state out of the workload's reach, out
  of diagnostics and out of anything serialized for users. `Meta` holds
  `proxy_token`, so never print or export it as is.
- The command surface is closed except through an OpenSpec change. The MVP
  set is `init`, `up`, `down`, `rm`, `status`, `logs`, `ps`, `shell`, `exec`,
  `ssh`, `proxy`, `ssh-config`, `policy`, `why` and `doctor`. Active changes
  add `agent …`, `trust` and `--file-only`, and the surface 1.0 keeps is listed
  in `docs/stability.md` (`prepare-v1-release`). `--help` and `--version` describe the surface; they don't add
  to it. Add a new command or flag to `USAGE` in `src/bin/devcroft.rs`, the
  only place a user of the published binary can discover it, or
  `tests/cli_help_and_version.rs` fails.
- Use `Result` for expected failures. Write a `SAFETY` comment for each new
  `unsafe` block. Follow the nearby code, and test behavior at the layer that
  owns it.
- Put a test seam behind the non-default `test-support` feature. Never use
  `cfg(test)` for it, because `tests/` compiles the crate as a dependency.
  Never let a seam reach a default build.
- Packaging is an anchored `include` allowlist in `Cargo.toml`. Add a file the
  crate needs with a leading `/`. Keep `!/src/bin/spike.rs`, or
  `cargo install devcroft` installs a second binary. Check what an install puts
  on `PATH`, not only the file count.
- After any `Cargo.lock` change, regenerate `THIRD-PARTY-LICENSES.md`.
- russh is the fork `devcroft-russh` in `vendor/russh`, published before
  devcroft. Never swap it for a `[patch.crates-io]` override: `cargo package`
  drops it, and the published binary would silently use unpatched russh. See
  CONTRIBUTING.md, "Release".
- The standard runtime never takes an ambient host shell or toolchain. Shell
  and utilities come from devcroft's runtime or the qualified provider, and a
  provider never inherits `/usr/bin` or the host's `PATH`. The native
  Swift/Xcode provider breaks this rule, so `exclude-swift-from-standard-build`
  takes it out of the standard build, behind a non-default research feature.
  Don't extend it, don't build on it, and don't copy its native mode into
  another provider.
- Take techniques from other projects, never tools. The keeper never runs as a
  child of a separate sandboxing binary, and the process tier needs no external
  backend binary. Record what you take in [docs/prior-art.md](docs/prior-art.md).
- Framing rules:
  - Make no blanket security claims. There's one isolation tier, and it's
    accident protection.
  - Keep `closure` and `artifact` guarantees distinct wherever they appear.
  - Don't describe domain filtering as cooperative; it's enforced.
  - There's no `host` or `none` provider. Until `add-file-only-workflow`
    lands, there's no non-reproducible mode at all.
  - Describe designs as designs: manifest approval, file-only, Mode 2 and
    microVM aren't shipped until their changes land.

## OpenSpec

```text
openspec/
  config.yaml                    # schema and `context:` fed to agents
  changes/<change-id>/
    .openspec.yaml               # required; without it the CLI can't see the change
    proposal.md  design.md  tasks.md
    specs/<capability>/spec.md   # delta specs
  changes/archive/
  specs/<capability>/spec.md     # main specs, empty so far
```

- Base specs live in `openspec/changes/add-mvp-core/specs/`. Changes were
  archived without syncing, so `openspec/specs/` is empty. Use `ADDED` for new
  requirements, and amend another active change's requirement with an explicit
  task in your change.
- Delta specs use `## ADDED|MODIFIED|REMOVED|RENAMED Requirements`, then
  `### Requirement:`. Every requirement needs at least one `#### Scenario:`
  with exactly four `#` characters.
- Project context is in `openspec/config.yaml` under `context:`.
  `openspec/project.md` is legacy; don't recreate it.
- Validate every change you touch with `--strict`. To see a change's
  artifacts and how to write one:

  ```sh
  openspec status --change CHANGE_ID --json
  openspec instructions ARTIFACT_ID --change CHANGE_ID --json
  ```
- The workflows are skills in `.agents/skills/`, with Claude Code copies in
  `.claude/skills/`: propose, update, apply, archive, sync and explore.

| Capability (`add-mvp-core`) | Owns |
|---|---|
| `config` | `devcroft.toml` schema, discovery walk, validation |
| `policy` | Profile compilation, `policy --render`, `why` |
| `env-provider` | Provider trait, resolution, staleness, provider rejection |
| `lifecycle` | Keeper, `up`/`down`/`rm`, `status`/`logs`/`ps`, hooks |
| `exec` | `exec` and `shell` sessions, PTY, signals, exit codes |
| `ssh` | Embedded server, `proxy`, `ssh-config`, keys |
| `cli` | Command surface, name resolution, `init`, `doctor`, error contract |

## Ask before you do these

- Commit, push, publish (`cargo publish`, reserving the npm name), or open an
  issue or pull request on a third-party repository, such as the nono
  trust-module request (`use-nono-library` task 6.4).
- Commit `new/` or `new2/`. They're the owner's local notes. Cite their
  content in tracked files, never their paths.

## Where knowledge lives

| Document | Holds |
|---|---|
| [README.md](README.md) | What a developer needs to evaluate, install and use devcroft |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Setup, tests, OpenSpec workflow, dependencies, packaging, release |
| [docs/invariants.md](docs/invariants.md) | The reasoning and measurements behind every invariant here |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", with the six provider criteria |
| [docs/known-gaps.md](docs/known-gaps.md) | Every published limitation, measured |
| [docs/threat-model.md](docs/threat-model.md) | Which use case the isolation backs |
| [docs/roadmap.md](docs/roadmap.md) | What each release has to be true for |
| [docs/implementation-log.md](docs/implementation-log.md) | What was built, and what turned out to be wrong |
| [docs/prior-art.md](docs/prior-art.md) | Ideas taken from other projects, and from where |
| [docs/agents.md](docs/agents.md) | Coding agents measured inside a sandbox |
| [docs/base-runtime.md](docs/base-runtime.md) | Where a sandbox's shell and utilities resolve |
| [samples/README.md](samples/README.md) | What each sample demonstrates |

`README.md` is written for human developers, not for agents. Keep it short
and easy to scan: what devcroft is, how to install and use it, and links to
detail. Don't add agent instructions, rationale, measurements or history
there; they belong in this file, `docs/invariants.md` or the other docs. Check
every claim you add to it against the code.

Keep this file to durable rules. Put procedures in `CONTRIBUTING.md`,
requirements in OpenSpec, reasoning in `docs/invariants.md`, and dated evidence
in the document it belongs to.

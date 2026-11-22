## 0. Measurement gate — no code until these are answered

Record results in `design.md` under "Evidence", tagged as measured, with harness versions and targets (linux-aarch64, osx-arm64), raw runs under `evidence/`.

- [ ] 0.1 Pick two candidates at pinned versions: first, one harness with native ACP shipped as a single binary; second, Claude Code through its ACP adapter (npm, Node). For each, record every artifact needed to reach an ACP `initialize` and one edit-and-run task, with URL and checksum per platform. Note any universal Mach-O and whether a thin artifact exists.
- [ ] 0.2 For each candidate, list every executable its chain starts and how it finds it (absolute path, `PATH`, `process.execPath`, a variable). Name the lever that pins each to an absolute path. Run with a sentinel `node`, `sh` and `git` first on the project's `PATH` and record which are hit.
- [ ] 0.3 For the npm adapter: lay out `node_modules/` from its lockfile without scripts (design A3: SRI, `package/` prefix, `os`/`cpu`/`libc`, `.bin` links, sanitizing) and confirm it starts. Record entries with `hasInstallScript` or platform-specific optionals, and whether any is needed.
- [ ] 0.4 Record what each candidate downloads or writes after install (self-update, lazy components, telemetry, config location, files written straight into `$HOME`, `XDG_*` use) and the switch that turns each off. Confirm the agent works with its object read-only and only the sandbox's `HOME` writable.
- [ ] 0.5 Node 16 project, Node 24 agent (where the candidate has one): the agent's shell tool reports the project's `node`; the agent process reports its own. Repeat with the project setting `NODE_OPTIONS`, and record what breaks.
- [ ] 0.6 Launcher (design A5): a runtime-shebang launcher inside the object runs on both platforms at the real store path length, and on macOS under exec scoping.
- [ ] 0.7 Auth: the broker template reaches the API with the project's `network.allow = []`. Subscription login: whether the flow completes inside the sandbox (loopback redirect versus paste code), on both platforms.
- [ ] 0.8 Denial hook (design A10): the harness reads a user-level hook from the sandbox's `HOME`; record whether a project-level setting can override it.
- [ ] 0.9 Editor: time from `agent acp` to `initialize` response on a cold sandbox with the object installed, for one editor's default timeout.
- [ ] 0.10 Stop criteria: report to the owner before group 1 if a candidate's chain starts an executable that cannot be pinned (0.2), cannot run from a read-only install (0.4), or the launcher cannot avoid a shell (0.6). If the npm adapter cannot be laid out without running scripts (0.3), or the layout proves too costly to maintain, decide between confined provisioning and dropping npm adapters from the first slice.

## 1. Recipes

- [ ] 1.1 `src/agent/recipe.rs`: recipe schema (design A2) with versions, parsed and validated at build time from `src/agent/recipes/*.toml`; placeholders resolve only inside the object; `[launch.env]` refuses `PATH`, `HOME`, `XDG_*`, `LD_*`, `DYLD_*` and `requires.runtime_env` names.
- [ ] 1.2 User recipes from `~/.config/devcroft/agents/`, same validator, checksums mandatory, precedence over an embedded recipe of the same name, marked as user recipes.
- [ ] 1.3 The recipes from group 0, `claude-code` among them.

## 2. Selection

- [ ] 2.1 `src/config/user.rs`: `[agent] name`, `version`; unknown keys refused; `[agent]` in `devcroft.toml` refused with a pointer to the user configuration; `up --agent`, `up --no-agent`; unknown recipe or version and unsupported platform fail at layer `config`.
- [ ] 2.2 Test: provider fingerprint and captured environment identical with and without an agent.

## 3. Materialization

- [ ] 3.1 Download and verify; extract `single-binary`, `archive`, `node-tarball` with path and link sanitizing; generate the launcher into the staging object.
- [ ] 3.2 `npm-lock` layout from lockfile v3 without scripts (design A3), if group 0 kept it.
- [ ] 3.3 Publish through `provider::artifacts` under `agents/`: per-object lock, staging, completion record; `agents` lease for all of `up` whenever an agent is selected; sweep after.
- [ ] 3.4 `devcroft agent install <recipe>` with progress on stderr.
- [ ] 3.5 Confined-provisioning install path for recipes that need it, if group 0 admitted one.

## 4. Launch and policy

- [ ] 4.1 Composed policy (design A11): agent rules folded in before `check_brokers`, the egress-proxy decision, rendering and compilation.
- [ ] 4.2 Recipe broker template active only for the selecting sandbox; conflicts with user broker entries refused; `network.default = "allow"` refused with `--no-agent` hint.
- [ ] 4.3 `[launch.env]` in the sandbox environment; `requires.runtime_env` warning; launcher directory on `PATH` after the project and the tooling layer, shadowing reported.
- [ ] 4.4 `[requires]` checks (design A13).
- [ ] 4.5 Denial hook wiring into the sandbox's `HOME`, if group 0 qualified it.
- [ ] 4.6 `Meta` records recipe, version, source and object id; staleness.
- [ ] 4.7 Add `agent:<recipe>` to CLAUDE.md's origin list.

## 5. CLI

- [ ] 5.1 Split `cli_agent` into host and in-sandbox verbs (design A7); `USAGE` lists both.
- [ ] 5.2 `agent list` (recipes, versions, platforms, auth variable, installed or not), `agent acp`, `agent run` (design A8); `maybe_auto_up` loads the user configuration.
- [ ] 5.3 `status` and `doctor` lines (spec `cli`).

## 6. Tests, docs

- [ ] 6.1 `tests/agent_packages_e2e.rs`: checksum mismatch, sentinel package, archive escape, shared object, sweep, lease held under a non-store project provider, Node 16 / Node 24, sentinel on project `PATH`, ACP `initialize` round trip with clean stdout, `cwd = /` refusal, absent object fails fast, wrong agent refusal, two checkouts, brokered request on a project with `network.allow = []`, rendered `agent:` rules, staleness.
- [ ] 6.2 `docs/agents.md` recipe column; `docs/decisions.md` §1 pointer from the "host's binary" rejection; `README.md`; `docs/known-gaps.md` (shared policy, environment and `HOME` until `add-agent-executor`); `add-agent-workload`'s credentials text amended for recipe hosts.
- [ ] 6.3 Fleet: a separate group, written after the `up` path is qualified; until then `fleet up` with an agent selected fails visibly rather than ignoring it.

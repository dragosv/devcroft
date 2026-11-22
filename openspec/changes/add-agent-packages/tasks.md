## 0. Measurement gate — no code until these are answered

Record results in `design.md` under "Evidence", tagged as measured, with harness versions and targets (linux-aarch64, osx-arm64), raw runs under `evidence/`.

- [ ] 0.1 Pick two candidates at pinned versions: one harness with native ACP shipped as a single binary, and Claude through its ACP adapter (npm, Node). For each, record every artifact needed to reach an ACP `initialize` and one edit-and-run task, with URL and checksum per platform.
- [ ] 0.2 For each candidate, list every executable its chain starts and how it finds it (absolute path, `PATH`, `process.execPath`, a variable). Name the lever that pins each to an absolute path. Run with a sentinel `node`, `sh` and `git` first on the project's `PATH` and record which are hit.
- [ ] 0.3 For the npm adapter: lay out `node_modules/` from its lockfile without scripts (design A3) and confirm it starts. Record entries with `hasInstallScript` or platform-specific optionals, and whether any is needed.
- [ ] 0.4 Record what each candidate downloads or writes after install (self-update, lazy components, telemetry, config location) and the switch that turns each off. Confirm the agent works with its object read-only and only its state directory writable.
- [ ] 0.5 Node 16 project, Node 24 agent (where the candidate has one): the agent's shell tool reports the project's `node`; the agent process reports its own.
- [ ] 0.6 Stop criteria: report to the owner before group 1 if a candidate's chain starts an executable that cannot be pinned (0.2), cannot run from a read-only install (0.4), or the npm adapter cannot be laid out without running scripts (0.3). In the last case, decide between confined provisioning and refusing npm adapters in the first slice.

## 1. Recipes

- [ ] 1.1 `src/agent/recipe.rs`: recipe schema (design A2), parsed and validated at build time from `src/agent/recipes/*.toml`; placeholders `{runtime}`, `{harness}`, `{adapter}`, `{state}` resolved only to paths inside the object or the state directory.
- [ ] 1.2 The two recipes from group 0.

## 2. Selection

- [ ] 2.1 `src/config/user.rs`: `[agent] name`; `up --agent`; unknown recipe and unsupported platform fail at layer `config`.
- [ ] 2.2 Test: provider fingerprint and captured environment identical with and without an agent.

## 3. Materialization

- [ ] 3.1 Download and verify; extract `single-binary`, `archive`, `node-tarball`.
- [ ] 3.2 `npm-lock` layout from lockfile v3 without scripts (design A3).
- [ ] 3.3 Publish through `provider::artifacts` under `agents/`: per-object lock, staging, completion record, lease while `up` runs, sweep at `rm` and `up --recreate`.
- [ ] 3.4 Confined-provisioning install path for recipes that need it, if group 0 admitted one.

## 4. Launch

- [ ] 4.1 Agent environment: project environment, plus `[launch.env]`, plus the broker placeholder; nothing prepended to `PATH`.
- [ ] 4.2 Launcher at `<state>/<name>/agent/bin/<command>`, on `PATH` after the project and the tooling layer; shadowing reported.
- [ ] 4.3 State directory per sandbox and recipe; removed at `rm`.
- [ ] 4.4 Grants with origin `agent:<recipe>`: object, host libraries by header walk, state directory, hosts. Add the origin to CLAUDE.md's list.

## 5. CLI

- [ ] 5.1 `devcroft agent list`, `devcroft agent acp [name]` (auto-up, stdio passthrough, stderr separate); both in `USAGE`.
- [ ] 5.2 `status` and `doctor` lines (spec `cli`).

## 6. Tests, docs

- [ ] 6.1 `tests/agent_packages_e2e.rs`: checksum mismatch, sentinel package, shared object, sweep, Node 16 / Node 24, sentinel on project `PATH`, ACP `initialize` round trip with clean stdout, state outside object and workspace, rendered `agent:` rules.
- [ ] 6.2 `docs/agents.md` recipe column; `docs/decisions.md` §1 pointer from the "host's binary" rejection; `README.md`; `docs/known-gaps.md` (shared policy until `add-agent-executor`).
- [ ] 6.3 Fleet: a separate group, written after the `up` path is qualified; until then `fleet up` with an agent selected fails visibly rather than ignoring it.

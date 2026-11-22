## 0. Measurement gate — no code until these are answered

Record each result in `design.md` under "Evidence", tagged as measured, with the mise version and target.

Progress on 2026-11-17: the candidate 2026.11.9 macos-arm64 and reproducible
sentinel runner are recorded in design.md's qualification section and
`evidence/install-macos-arm64-2026.11.9.json`. Core Node/Go/Python execute their
downloaded binaries; the synthetic HTTP/GitHub cases do not. This is partial
0.1/0.2 evidence, not completion or a failed stop criterion for every backend.
The final seven fixtures cover aqua and conda as well, with no payload execution
in those cases. Linux execution and real-build qualification remain outstanding;
group 1 has not started. The source-derived discovery inventory closes 0.3.

- [x] 0.1 Pin the qualified mise version. For each candidate backend (core archive tools, aqua, github, http, conda), install a fixture through the derived tools-only configuration under the allowlisted environment of design M2, where the downloaded archive's executables write a marker when run. Record which backends execute anything they downloaded, and which setting, if any, switches that step off. Record the exact names of: the data, config, state, cache and system directory variables, the global-config variable, the trusted-config setting, the setting that stops the parent-directory walk, and every file mise reads from `HOME`.
- [x] 0.2 Measure the lock: which name pairs with `.mise.toml`; which fields each admitted backend writes per platform on `linux-x64`, `linux-arm64`, `macos-arm64` and `macos-x64`; whether `mise install` under locked mode leaves the copied lock and `.mise/locks/` sidecars byte-identical. A backend that rewrites the lock is refused. Candidates without real lock-generation/runtime evidence remain unadmitted; the initial admitted set is HTTP only (2026-11-20).
- [x] 0.3 Enumerate, from the pinned version's own discovery order, every project-level configuration location and idiomatic version file mise loads. This list replaces the candidates in design M1.
- [x] 0.4 On Linux (glibc, and musl if a profile is wanted) and macOS, under `own-policy-baseline`: measure the files the host shell needs when granted as a file; confirm `cc`, `gcc`, `ld` and `git` from the host do **not** resolve inside the sandbox; run a real build with a compiler declared in `[tools]`. Decide the header reader (hand-written or the `object` crate) and state the dependency count it adds.
- [x] 0.5 Stop criterion. If fewer than two of Node, Python and Go are installable through an admitted backend on Linux and on macOS, or if the narrow sandbox of 0.4 cannot complete a real build, stop and report to the owner before group 1.

## 1. Provider selection and bounded inputs

- [x] 1.1 In `src/provider/validate.rs` move `"mise"` from `NOT_YET_SUPPORTED` to `SUPPORTED`; rewrite the unit test that asserts mise is not yet supported to use `hermit`. Amend the "Only declarative providers" scenario in `openspec/changes/add-mvp-core/specs/env-provider/spec.md` the same way, so two active changes do not disagree.
- [x] 1.2 Add `ProviderKind::Mise` (`Tier::Artifact`) in `src/provider/mod.rs` with arms in `from_name`, `static_name`, `tier`, `resolution_reach` and `manifest_fingerprint`. `host` and `none` stay refused at layer `config`.
- [x] 1.3 Root manifest discovery: one of `mise.toml` or `.mise.toml`; two is a refusal naming both. Refuse by path every other location from 0.3, at layer `provider`, before any subprocess.
- [x] 1.4 Table classification per design M1: `[tools]` and `[env]` consumed; `[settings]` allowlist with every other key refused by dotted key whatever its value; `[hooks]`, `[watch_files]`, `[plugins]`, `[alias]`, `[tool_alias]` and unknown tables refused; `[tasks]`, `[task_config]`, `[vars]` not carried; `[daemons]` refused by name unless `--skip-hooks`; `min_version` checked.

## 2. Lock verification and the artifact store

- [x] 2.1 Parse the lock and sidecars in `src/provider/mise.rs`. Before any subprocess, require an admitted backend and a URL plus checksum for the current platform for every selected tool; refuse by tool, backend and reason at layer `provider`.
- [x] 2.2 Build the subprocess environment from the allowlist of design M2 (`env_clear`, isolated `HOME`, canonical `PATH`, every mise directory inside the object, locked mode, forwarded proxy and CA variables only). Test that an ambient `MISE_ENV`, `MISE_LOCKED_REQUIRE_CHECKSUM=0` and a default-packages file in the real `HOME` change nothing.
- [x] 2.3 Write the derived directory: tools-only `mise.toml`, byte copies of the lock and sidecars.
- [x] 2.4 Store at `~/.local/share/devcroft-artifacts/mise/<object-id>/`: object id per design M4; exclusive per-object lock; remove a directory that has no completion record before installing; install with mise's data directory at `<object>/data`; verify lock bytes unchanged; write the completion record atomically.
- [x] 2.5 Record the object id in `Meta` (`src/lifecycle/state.rs`). At `rm`, and at `up --recreate` when the id changes, remove every mise object no recorded `Meta` names, under the exclusive store lock; `up` holds the shared lock from acquiring its object until its `Meta` is written.
- [x] 2.6 In `src/policy/capability_set.rs`, beside `package_manager_authority`, refuse a resolved manifest write grant that contains or is contained by a provider artifact root, at layer `config`, with a message naming the store.

## 3. Environment capture

- [x] 3.1 Capture the tool environment with `mise env --json` against the derived directory, diffed against `capture::canonical_base_env()`.
- [x] 3.2 Render the project's `[env]` into `Resolution::activation_script` in declaration order per the table in design M3, anchored at the manifest's directory, and refuse every directive outside that table by key.
- [x] 3.3 `fingerprint` over the manifest, lock, sidecars, dotenv and source files, adapter version and target, using `capture::optional_file_part` for every file but the manifest.

## 4. Grants and the shell

- [x] 4.1 Populate `Resolution::read_only_grants` with `<object>/data/installs` and the profile's host-runtime inventory, origin `provider:mise`.
- [x] 4.2 Shell: from the object if a tool ships one; else on Linux the canonicalized host shell file plus its loader and libraries as file grants, on macOS `host_shell_dir()` and `/var/select`. No `/usr/bin` grant on either platform.
- [x] 4.3 Precondition check: read each installed executable's interpreter and needed-library names with the reader chosen in 0.4; fail at layer `provider` naming the tool and the library when one resolves neither inside the object nor in the inventory. Never execute the binary to find out.

## 5. CLI

- [x] 5.1 `init`: detect a mise manifest by reading files only.
- [x] 5.2 `doctor`: mise binary and version against the qualified range, lock coverage for this platform, backends that would be refused, the shell that would be resolved, store size, and that `[tasks]` are not runnable in the sandbox. Installs nothing.
- [x] 5.3 `status`: replace the hardcoded "flake" / "flox manifest" staleness wording in `src/bin/devcroft.rs` with the active provider's manifest name.
- [x] 5.4 Minimum mise version enforced; one warning for a version newer than the last qualified.

## 6. Tests, sample, documentation

- [x] 6.1 Refusal tests: competing manifests, each extra config location, each refused table and setting, unadmitted backend, missing URL or checksum, template other than `{{config_root}}`, `[daemons]` with and without `--skip-hooks`.
- [x] 6.2 Materialization tests: cold locked restore, lock unchanged, interrupted install removed before reinstall, two consumers of one object, object survives one `rm` and is removed by the last, concurrent `up` and sweep.
- [ ] 6.3 Policy tests: runtime write to an install refused, overlapping manifest write grant refused at `config`, host `cc` unresolvable inside, shell resolved from the file grant on Linux. File the pre-existing `sandbox_identity` exact-match observation as its own issue; do not fix it here. *(2026-11-22: write refusal, overlap refusal and host `cc` denial pass on macOS ARM64 in `tests/mise_provider_e2e.rs`; the Linux shell file grant is asserted by the same test on Linux ARM64 but was measured only in group 0, not re-run here. Filing the issue is an external action left to the owner.)*
- [x] 6.4 Environment tests: `[env]` order with a literal after a `_.source`, relative `_.path` and `{{config_root}}` anchored at the project, staleness on an edited and on a deleted source file, caller credentials absent from the captured environment.
- [x] 6.5 `samples/mise-sample`: a locked build with a declared compiler, with a README that shows the artifact-tier line, `policy --render` and the `mise run` limitation.
- [x] 6.6 Once the above pass: `docs/decisions.md` §1, `README.md`, `docs/known-gaps.md`, `openspec/config.yaml`, and the framing rule in `CLAUDE.md` that lists mise as not yet supported.

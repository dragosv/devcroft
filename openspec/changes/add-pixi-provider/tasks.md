## 0. Measurement gate — no code until these are answered

Record each result in `design.md` under "Evidence", tagged as measured, with the pixi version and target.

2026-11-22: 0.1–0.5 measured with pixi 0.84.2 on osx-arm64 and linux-aarch64 (design.md, "Group 0 measurement"; `evidence/`). Stop criterion 0.6 applies in its second branch: Conda-only installation runs nothing it downloaded, PyPI installation runs the environment's interpreter, so the first slice is Conda-only. Two further owner decisions come from the measurement: the keychain host allowlist and the precondition scope (`wish8.6`). Owner decisions, 2026-11-22: (1) Conda-only first slice; (2) lock URLs restricted to `conda.anaconda.org` and `repo.prefix.dev`; (3) an executable needing a host library outside the fixed inventory is warned about once and not granted, never refused.

- [x] 0.1 Pin the qualified pixi release and confirm `install --locked --environment` and `shell-hook --shell bash --as-is --no-completions --environment --manifest-path` exist on it. Against a fixture whose package `etc/conda/activate.d/*.sh` and project activation scripts write markers: confirm the text form of `shell-hook` leaves every marker untouched, installs nothing and leaves `pixi.toml` and `pixi.lock` byte-identical. Record the exact switch and variables that isolate pixi from user and system configuration and from `HOME`.
- [x] 0.2 Measure the lock on `linux-64`, `linux-aarch64`, `osx-arm64` and `osx-64`: the format version or versions the pinned pixi writes, and which fields distinguish a binary Conda package and a locked wheel from an sdist, an editable, a path, a VCS and a `pixi-build` entry. Confirm what `pixi init --format pyproject` writes for the project's own dependency.
- [x] 0.3 With a sentinel interpreter (a package whose `python` writes a marker when run), measure whether `pixi install --locked` executes anything it downloaded: once for a Conda-only environment, once for one with PyPI wheels. Record whether prefix files are hardlinks, clones or copies of cache objects.
- [x] 0.4 Measure the activation rewrite of design P4 against a fixture with workspace, feature-level and target-level activation scripts and `[activation].env`: order preserved, `PIXI_PROJECT_ROOT` and script paths on the real project, no staging path left outside the prefix. Record whether pixi requires the scripts to exist when it emits the hook. Measure which keys of a project `.pixi/config.toml` are safe to admit.
- [x] 0.5 On Linux and macOS under `own-policy-baseline`: the files the host `sh` and `bash` need when granted as files; that the host's `cc`, `gcc` and `git` do **not** resolve inside the sandbox; a real Python plus native-extension workload and a compiler build from the environment alone.
- [x] 0.6 Stop criteria. If the text hook is not inert on any release, or a Conda-only install executes downloaded code, stop and report to the owner. If only PyPI-bearing environments do, the first slice is Conda-only: say so to the owner before group 1 and keep the PyPI refusal in 2.1.

## 1. Config, selection and the provider seams

- [x] 1.1 `src/provider/validate.rs`: move `"pixi"` from `NOT_YET_SUPPORTED` to `SUPPORTED`. `src/provider/mod.rs`: add `ProviderKind::Pixi` (`Tier::Artifact`) with arms in `from_name`, `static_name`, `tier` and `resolution_reach`.
- [x] 1.2 `src/config/{mod,validate}.rs`: optional `[env.pixi]` with `manifest` and `environment`; unknown keys and `[env.pixi]` under another provider fail at layer `config`.
- [x] 1.3 Thread the selection: `Provider::resolve`, `manifest_fingerprint` and `is_stale` in `src/provider/mod.rs` receive it; `Meta` in `src/lifecycle/state.rs` persists it with the object id; `status`, `policy --render` and `why` reconstruct from `Meta`. No ambient `PIXI_*` variable is read anywhere. *(Implemented differently, design "Implementation notes": the selection is read from `devcroft.toml` by `provider::pixi`; `Meta` persists the object id, not the selection.)*
- [x] 1.4 Workspace discovery: `pixi.toml`, or `pyproject.toml` with `[tool.pixi]`; both without a selector is a refusal naming both; a selector must be a regular file inside the canonical project root.

## 2. Locked materialization and the artifact store

- [x] 2.1 In `src/provider/pixi.rs`, parse the manifest, the lock and the project's `.pixi/config.toml`. Before any subprocess: require a covered lock version and a solution for the selected environment and platform; check recorded system requirements against the host; refuse by package and reason every `pypi:` entry (0.3: installing it runs the environment's interpreter), `conda_source:` entries, path locations, and lock versions other than 7; refuse package URLs whose host is not `conda.anaconda.org` or `repo.prefix.dev`; refuse config keys outside the 0.4 allowlist by dotted name, `run-post-link-scripts = "insecure"` among them.
- [x] 2.2 Build the subprocess environment from the allowlist of design P2. Test that a user-level pixi config, an ambient `PIXI_ENVIRONMENT_NAME` and credentials in the real `HOME` change nothing.
- [x] 2.3 Store at `~/.local/share/devcroft-artifacts/pixi/<object-id>/`: object id per design P3; exclusive per-object lock; remove a directory with no completion record before installing; stage byte copies of manifest, lock and admitted config; `pixi install --locked --environment <env>`; verify manifest and lock unchanged; write the completion record atomically.
- [x] 2.4 At `rm`, and at `up --recreate` when the id changes, remove every pixi object no recorded `Meta` names, under the exclusive store lock; `up` holds the shared lock from acquiring its object until its `Meta` is written.
- [x] 2.5 In `src/policy/capability_set.rs`, beside `package_manager_authority`, refuse a resolved manifest write grant that contains or is contained by a provider artifact root, at layer `config`. Shared with `add-mise-provider` task 2.6; whichever lands first adds it.

## 3. Activation, grants and staleness

- [x] 3.1 Capture the hook as text, apply the staging-to-project rewrite of design P4, and assemble `Resolution::activation_script` as: `cd` to the project root, the rewritten text, `unset PIXI_EXE`. Refuse activation script paths that are absolute or leave the project.
- [x] 3.2 Populate `Resolution::read_only_grants` with `<object>/.pixi/envs/<env>` and the profile's host-runtime inventory, origin `provider:pixi`.
- [x] 3.3 Shells: `sh` and `bash` from the prefix; else on Linux the canonicalized host files plus their loader and libraries as file grants, on macOS `host_shell_dir()` and `/var/select`. No `/usr/bin` grant on either platform. *(One POSIX shell suffices, design "Implementation notes"; no separate Bash grant.)*
- [x] 3.4 `fingerprint` over the manifest, lock, environment name, project config, activation scripts and adapter version, using `capture::optional_file_part` for every file but the manifest.
- [x] 3.5 Return `ServiceSupport::Unsupported`.

## 4. CLI

- [x] 4.1 `init`: detect a Pixi workspace by reading TOML only.
- [x] 4.2 `doctor`: pixi binary and version against the qualified range, manifest, lock and platform, packages that would be refused, store size, and that tasks are not runnable by name in the sandbox. Installs nothing.
- [x] 4.3 `status`: selected environment and tier; replace the hardcoded "flake" / "flox manifest" staleness wording in `src/bin/devcroft.rs` with the active provider's manifest name (shared with `add-mise-provider` task 5.3). *(The staleness line names the pixi inputs; `status` shows the tier but not the environment name.)*
- [x] 4.4 Minimum pixi version enforced; one warning for a version newer than the last qualified.

## 5. Tests, sample, documentation

- [x] 5.1 Config and refusal tests: `[env.pixi]` validation and provider mismatch, competing manifests, escaping selector, each refused dependency kind, the default pyproject layout, each refused config key, unknown lock version.
- [x] 5.2 Materialization tests: cold locked restore, inputs unchanged, interrupted install removed before reinstall, two sandboxes of one checkout on one object with separate homes, object survives one `rm` and is removed by the last, concurrent `up` and sweep.
- [ ] 5.3 Policy tests: runtime write to the prefix refused, overlapping manifest write grant refused at `config`, host `cc` unresolvable inside, shells resolved from file grants on Linux. File the pre-existing `sandbox_identity` exact-match observation as its own issue; do not fix it here. *(2026-11-22: the tests pass on osx-arm64 and linux-aarch64, in `tests/pixi_provider_e2e.rs`; filing the issue is left to the owner, as in `add-mise-provider` 6.3.)*
- [x] 5.4 Activation tests: host-side sentinels untouched, order across workspace, feature and target scripts, `PIXI_PROJECT_ROOT` on the real project, provisioning network not inherited by sessions, staleness on an edited and on a deleted script. *(Host-side sentinels and cross-level order are Group 0 evidence and a rewrite unit test; the provisioning-network split is the shared mechanism already covered by `tests/provisioning_network.rs`, not re-tested for pixi.)*
- [x] 5.5 Service tests: `[network.services.<name>]` under pixi fails at `config`; a flox manifest with services beside a pixi selection fails at `provider`.
- [x] 5.6 `samples/pixi-sample`: a locked workspace with a native extension, with a README showing the artifact-tier line, `policy --render`, what is refused and the `pixi run` limitation.
- [x] 5.7 Once the above pass: `docs/decisions.md` §1, `README.md`, `docs/known-gaps.md`, `openspec/config.yaml`, and the framing rule in `CLAUDE.md` that lists pixi as not yet supported.

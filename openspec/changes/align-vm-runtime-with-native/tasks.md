## 1. Correct the record first

- [ ] 1.1 Reopen each `add-vm-project-runtime` task whose cited function the `up --vm` path does not call (2.5, 2.6, 3.1-3.6 as of 2026-11-28), and rewrite the evidence of the rest to cite the CLI path. Review finding 1.
- [ ] 1.2 Add the 2026-11-23 hvi stop runs (two `panic-without-stop-intent` failures in nine with the bounded helper, none in eight with the old one) to `results/macos-arm64-hvi-pending-bound.json`, and keep the row's status scoped until they are explained.
- [ ] 1.3 Record the workspace-model decision (design D6) and the `network.allow` grant default in `docs/decisions.md`, with the owner's answer. Blocks group 6.

## 2. Authority: render equals enforcement

- [ ] 2.1 Compile `CompiledVmAuthority` once in `up --vm` and derive the proxy allowlist, broker routes, delivered values and launch record from it. Review finding 2.
- [ ] 2.2 Make `policy --render` render that value; delete the grant set `render_vm_policy_plan` builds from requests; test that render, proxy allowlist and launch record are equal.
- [ ] 2.3 Extend `RuntimeGrantStore` with egress hosts and forwarded names bound to project identity and definition hash, plus one command that shows and accepts the current request set.
- [ ] 2.4 Refuse `[env] forward` and host-referencing `[env.vars]` without a grant; list every host-sourced value delivered at `up`; test with a forwarded secret, granted and not. Review finding 3.

## 3. Transfers out

- [ ] 3.1 Add the refused set to `sync-out` (`devcroft.toml`, `.devcroft/`, `.envrc`, `.git/`, editor task/launch files, code-running build configuration, new executable modes) with quarantine and named output; one test per entry. Review finding 4.

## 4. Environment from the provider

- [ ] 4.1 Make provider capture, shell resolution (`src/shell.rs`) and base-name provenance take a target system, without changing native output (native suite and provenance tests unchanged).
- [ ] 4.2 Wire `preparation.rs` into `up --vm`: start a preparation VM, resolve the manifest's provider for `aarch64-linux`, capture the profile. Review finding 5.
- [ ] 4.3 Wire `publication.rs`: publish the closure disk under the project namespace and attach it read-only at the native store path.
- [ ] 4.4 Refuse by name a lock without `aarch64-linux`, an unqualified provider, and a Darwin provider, at layer `provider`; distinguish "not yet supported in VM mode" from "out of scope".
- [ ] 4.5 Requalify the first provider for the guest (order per design open question) and pass a real `up --vm` for its sample.
- [ ] 4.6 Requalify the remaining closure-tier providers (`nix`, `flox`, `devbox`, `devenv`), each with a real `up --vm` of its sample.
- [ ] 4.7 Requalify the artifact-tier providers (`mise`, `pixi`): install for `linux-aarch64` inside preparation, publish the install, keep the `artifact` label, refuse a lock without Linux artifacts by name.
- [ ] 4.8 Resolve the tooling layer (`--tools`, `[tools]`) for the guest and compose it after the project's environment; refuse an unresolvable layer by name.
- [ ] 4.9 Test that every accepted provider either starts in VM mode or is refused by name, with `swift` always refused.

## 5. Shell and userland parity

- [ ] 5.1 Build devcroft's multi-call runtime (uutils and brush) for `linux-arm64` into the guest binary, at the host's runtime generation.
- [ ] 5.2 Keep the guest image's own base off session `PATH`s and out of resolution candidates.
- [ ] 5.3 Resolve the guest shell in the native order and use it for SSH login, `shell` fallback and service commands.
- [ ] 5.4 Report the per-name provenance record for VM instances at `up` and `status`.
- [ ] 5.5 Add a parity test: the same sample natively and in a VM, every base name in both lookup forms run in a real guest, provenance kinds compared.

## 6. Workspace

- [ ] 6.1 Implement the decided model (1.3); for option 2: base manifest, three-way comparison, deletions, conflict stop, `.git` included, streaming limits. Review finding 7.
- [ ] 6.2 Map the host working directory to the guest path in `exec`, `shell` and `ssh`. Review finding 9.
- [ ] 6.3 Make `rm` and `up --recreate` list untransferred guest changes and require confirmation or `--yes`. Review finding 8.
- [ ] 6.4 Refuse restarting an instance created with a bundle workload, telling the user to transfer work out and recreate.

## 7. Structure

- [ ] 7.1 Add one instance resolver returning native or VM, and replace per-command `load_vm_instance` branching in `src/bin/devcroft.rs`. Review finding 6.
- [ ] 7.2 Split `src/vm/cli_runtime.rs` into lifecycle, authority, workspace and rendering modules.

## 8. CLI surface and docs

- [ ] 8.1 Settle the spelling (`--runtime <process|vm>`, `--vm` alias) and regroup `runtime` subcommands by purpose. Review finding 11.
- [ ] 8.2 List every VM command and flag in `USAGE` and command usage lines; extend `tests/cli_help_and_version.rs` to subcommands and flags. Review finding 10.
- [ ] 8.3 Make missing-bundle and unqualified-host errors name the next command; add VM checks to `doctor`.
- [ ] 8.4 Show provider, lock identity, shell provenance and workspace state in `status` for VM instances. Review finding 12.
- [ ] 8.5 Describe VM mode, its differences from native and `DEVCROFT_VM_ROLLBACK` in README, `docs/known-gaps.md`, `docs/threat-model.md`, `docs/base-runtime.md` and `AGENTS.md`. Review finding 12.

## 9. Merge gate

- [ ] 9.1 Run the native suite, the VM parity test and the VM e2e tests on macOS ARM64 with no skips, and record the run.
- [ ] 9.2 Confirm every review finding is fixed or recorded as a decided limit in `docs/decisions.md` before merging `microvm` into `dev`.

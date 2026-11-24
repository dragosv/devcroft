## 0. Owner decisions and measurement gate

- [x] 0.1 Owner: confirm the reading of "the process tier requires no external backend binary" that admits `newuidmap`/`newgidmap` as host identity helpers (design D2), or decide that `separate` is not the Linux default.
  - Confirmed by the owner 2026-11-24: `newuidmap`/`newgidmap` are the host's identity helpers, not a sandbox backend.
- [ ] 0.2 Owner: confirm that a launchd helper which is the `devcroft` binary itself satisfies "never a child of a separate sandboxing binary" (design D3), and accept a signed macOS package for `separate` mode.
  - Open; 2026-11-24 the recommendation is to defer (macOS stays `shared`).
- [ ] 0.3 Owner: may the manifest select `shared`, or only the user (design, open question)?
- [x] 0.4 Linux: measure, on a host with delegated ranges, a user namespace mapping a non-zero inside uid to a subordinate host uid through `newuidmap`, combined with the existing mount view, PID namespace and Landlock; record what breaks.
  - Works; design.md "Measured: Linux". With a prototype of the view, not devcroft's own.
- [ ] 0.5 Linux and macOS: measure ACL inheritance for a sandbox-created file edited afterwards by the user from VS Code, a JetBrains IDE and `git`; record `safe.directory` behaviour.
  - Linux, command line and git: **D5's ACLs do not keep the user's access** (an explicit mode masks it). `separate` as the Linux default waits for the owner. **Open:** macOS.
- [ ] 0.6 macOS: measure creating a hidden account, starting a process as it from a root helper with inherited descriptors, and Seatbelt applied after; record what Swift (native) loses under such an account.

## 1. Config and model

- [ ] 1.1 `[identity]` table (`user`, `mode`) with validation at layer `config`; `root`, unknown keys and host ids refused.
- [ ] 1.2 Identity model in `Meta`: inside name/uid/gid, mode, host uid/gid, allocation record; old `Meta` reads as "legacy".
- [ ] 1.3 `status` and `policy --render` show it.

## 2. The environment is read-only to the workload (both modes)

- [x] 2.1 Linux view: `.devcroft/<name>/` mounted read-only except `home/`.
  - Plus `run/` (design D4). Fleet agents too.
- [x] 2.2 macOS: Seatbelt write denial under `.devcroft/<name>/` except `home/`.
  - `(deny file-write* (require-all … (require-not …)))`; also refuses moving `.devcroft`.
- [x] 2.3 Tests: overwriting a runtime or namespace entry, the compiled profile and the redirect library fails from a session; the workspace and `HOME` stay writable.
  - `tests/runtime_namespace.rs`, both platforms, with the policy artifact, the `devcroft` link, `services.yaml` and new files. Suites 2026-11-23: macOS 767/0; Linux aarch64 `test-support` 854/2, the two in `fleet_init` failing without this change too.

## 3. Linux

- [ ] 3.1 `shared`: dedicated inside name and uid through the existing user namespace; `/etc/passwd` and `/etc/group` in the view name the dedicated identity; `$USER`, `$LOGNAME` set.
- [ ] 3.2 `separate`: subordinate-range allocation, `newuidmap`/`newgidmap`, sandbox `HOME` owned by the mapped id, keeper and every workload process under it; setup helper root only before restriction.
- [ ] 3.3 Fleet: agents and the init helper follow the same mapping; clones owned by the agent's identity.
- [ ] 3.4 Precondition checks and refusal at layer `backend` (exit 4) naming each missing item and `shared`; the same in `doctor`.
- [ ] 3.5 Workspace ACLs with inheritance on `up`, removed on `rm`; `safe.directory` in the sandbox's git configuration.
- [ ] 3.6 Keeper log and state through descriptors opened by `up`; state and data directories unchanged.

## 4. macOS

- [ ] 4.1 `shared` is the platform default until 4.2–4.5 ship; `up` states it.
- [ ] 4.2 The privileged helper (same binary, `SMAppService`), authenticated, with the narrow protocol of design D3.
- [ ] 4.3 Per-sandbox hidden accounts, created and released; bounded pool; listed by `doctor`.
- [ ] 4.4 Keeper started as the account with inherited listeners; Seatbelt applied by the keeper as today.
- [ ] 4.5 Workspace ACLs; Swift ordinary build and signing qualified, or `shared` documented as required for signing.
- [ ] 4.6 Signed package for the helper; documentation of installation and removal.

## 5. Tests

- [ ] 5.1 Every workload process (session, hook, service, agent) reports a non-zero uid and the dedicated name.
- [ ] 5.2 Split `tests/up_pid_namespace.rs` into the `shared` and `separate` contracts.
- [ ] 5.3 Two `separate` sandboxes get distinct host uids; a released id is not reused while files remain.
- [ ] 5.4 The refusal message names each missing precondition.
- [ ] 5.5 Provider-row suite in both modes on Linux; skip inventory recorded.

## 6. Documentation

- [ ] 6.1 `docs/decisions.md`: rewrite "`remoteUser`, UID/GID mapping → not applicable".
- [ ] 6.2 `docs/threat-model.md`: what a separate uid adds (DAC) and what it does not (a boundary).
- [ ] 6.3 `docs/known-gaps.md`: macOS `shared` limits; Swift under a separate account; ownership-sensitive tools.
- [ ] 6.4 `CLAUDE.md`: the identity model and the two modes.

## 7. Verification

- [ ] 7.1 `cargo build`, `fmt`, `clippy`, `doc --no-deps`, `test`.
- [ ] 7.2 `openspec validate add-sandbox-identity --type change`.

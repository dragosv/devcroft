## 1. Load once and identify the checkout

- [ ] 1.1 Make `config::load` open the manifest with `O_NOFOLLOW`, require a regular file with one link, and return the parsed manifest with the SHA-256 of the same bytes. Test symlinked, hard-linked, missing and unreadable manifests.
- [ ] 1.2 Implement approval records under `<data_dir>/trust/`, keyed by canonical root and the root directory’s `(st_dev, st_ino)`, with sticky `invalidated` and `revoked` states. Test two worktrees, a moved checkout and a re-clone at the same path.

## 2. Build one effective manifest

- [ ] 2.1 Introduce `EffectiveManifest` and the `NotApplied` list, and route preflight, the proxy decision, provider resolution, provisioning, environment capture, compilation, rendering and `Meta` through it, so no later phase can read raw grants. Place admission before the `validate-before-recreate` preflight.
- [ ] 2.2 Compile the project root as a base rule with origin `workspace`, independent of the `filesystem.allow` default. Render not-applied keys in `policy --render` and keep `why` consistent with the compiled profile. Refuse at layer `config` a combination that becomes invalid once keys are cleared.
- [ ] 2.3 Withhold the user’s `[credentials] env` and `[[credentials.broker]]` from untrusted checkouts, naming each, while still delivering a selected agent recipe’s credential.

## 3. Admit attachments

- [ ] 3.1 Record trust state, manifest hash and approval per generation in `Meta`. Apply the attach rule to `up` (already up), `exec`, `shell`, `ssh`, `proxy`, `agent run` and `agent acp`. Make `maybe_auto_up` resolve the checkout before looking up the sandbox, and refuse a root mismatch naming `--name`.
- [ ] 3.2 Route fleet’s manifest loading through the same admission and confirm a fleet agent gets no grant `up` would not give.

## 4. Protect the manifest

- [ ] 4.1 On Linux, mask `<root>/devcroft.toml` in `construct_view` with an unreadable read-only bind of a devcroft-owned empty file. On macOS, add Seatbelt denials for reading and writing it. Refuse `up` at layer `backend` where neither can be enforced.
- [ ] 4.2 Measure every alias: case variants on case-insensitive volumes, `/private` prefixes, symlinks inside the workspace, and hard links created inside the sandbox. Record whether the name is masked or absent per platform, and report it in `status`.
- [ ] 4.3 For an untrusted checkout, protect `.git/config`, `.git/hooks/`, an enabled `.git/config.worktree`, a worktree’s `.git` pointer and workspace hook paths such as `.husky/`, at the view’s workspace path and with case aliases. Never create a file in the host checkout except an empty `.git/hooks/`. Confirm that a planted hook, a `core.fsmonitor` entry and a rewritten pointer fail, and that `git commit` still works.
- [ ] 4.4 Inventory everything inside the sandbox that reads the manifest (in-sandbox `devcroft` verbs, the denial hook) and give it the effective configuration instead.

## 4b. Close the other routes

- [ ] 4b.1 Withhold `ssh.forward_agent` for untrusted checkouts. Confirm that no per-session option or overlay brings back a withheld variable, an agent socket or another host socket.
- [ ] 4b.2 Record absent filesystem grants in `NotApplied` with the reason `absent`, name them at `up`, and make `why` agree. Update the existing gap in `docs/known-gaps.md`.
- [ ] 4b.3 Write every admission refusal to `stderr` only. Confirm that `ssh NAME.devcroft` through `ProxyCommand devcroft proxy %n` shows the refusal and no protocol error.
- [ ] 4b.4 Refuse untrusted checkouts for nix, flox, devbox, devenv and mise, naming `devcroft trust` and `devcroft up --file-only`. Qualify pixi with sentinel values against its resolution, capture and provisioning; until it passes, it refuses too. Make `trust` state that approval lets the provider evaluate the repository’s definitions on the host.
- [ ] 4b.5 Keep `network.ports` and service ports on Linux with the sandbox’s own namespace; withhold `expose`, shared-loopback ports, `env.pixi.macos_sdk` and loader or trust-store `[env.vars]`. Write the checkout id at `trust`, protect it, and make attach checks leave the record unchanged.

## 5. Command surface

- [ ] 5.1 Implement `devcroft trust` and `trust --revoke` with the TTY prompt, the non-interactive `--yes --sha256` rule and the change summary against the earlier record. Make `init` say the checkout is untrusted until `devcroft trust`, approving nothing. Add both to `USAGE`, help and `tests/cli_help_and_version.rs`.
- [ ] 5.2 Show trust state, recorded and current hash, unapplied keys and manifest visibility in `status`.

## 6. Migrate and verify

- [ ] 6.1 Give the test helpers an approval-by-hash step, and add `devcroft trust` to every sample README whose manifest grants anything beyond the workspace. Confirm no e2e test now passes by silently losing a grant.
- [ ] 6.2 Run the acceptance cases on Linux and macOS separately:
  - read-write workspace and manifest protection, with an editable control file;
  - an untrusted manifest with filesystem, egress, forwarding, provisioning and a global credential, using sentinel values and destinations;
  - an approved manifest, a one-byte change, then `up`, auto-up and attach to a still-running trusted generation;
  - the same bytes in two worktrees;
  - a backend that cannot enforce protection, refusing.
- [ ] 6.3 Update `docs/threat-model.md`, `docs/known-gaps.md`, the README’s status, the policy-origin table (`workspace`) in `docs/invariants.md`, and `docs/roadmap.md`’s 1.0 section, which this change and the file-only workflow redefine.

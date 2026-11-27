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
- [ ] 4.3 Inventory everything inside the sandbox that reads the manifest (in-sandbox `devcroft` verbs, the denial hook) and give it the effective configuration instead.

## 5. Command surface

- [ ] 5.1 Implement `devcroft trust` and `trust --revoke` with the TTY prompt, the non-interactive `--yes --sha256` rule and the change summary against the earlier record. Make `init` approve the bytes it writes. Add both to `USAGE`, help and `tests/cli_help_and_version.rs`.
- [ ] 5.2 Show trust state, recorded and current hash, unapplied keys and manifest visibility in `status`.

## 6. Migrate and verify

- [ ] 6.1 Give the test helpers an approval-by-hash step, and add `devcroft trust` to every sample README whose manifest grants anything beyond the workspace. Confirm no e2e test now passes by silently losing a grant.
- [ ] 6.2 Run the acceptance cases on Linux and macOS separately:
  - read-write workspace and manifest protection, with an editable control file;
  - an untrusted manifest with filesystem, egress, forwarding, provisioning and a global credential, using sentinel values and destinations;
  - an approved manifest, a one-byte change, then `up`, auto-up and attach to a still-running trusted generation;
  - the same bytes in two worktrees;
  - a backend that cannot enforce protection, refusing.
- [ ] 6.3 Update `docs/threat-model.md`, `docs/known-gaps.md`, the README’s status, the policy-origin list (`workspace`) in `CLAUDE.md`, and `docs/roadmap.md`’s 1.0 section, which this change and the file-only workflow redefine.

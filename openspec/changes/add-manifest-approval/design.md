## Context

`config::load` reads `devcroft.toml` and the compiled policy follows it key for key. Two facts make that unsafe for a repository the user did not write:

- **The workload can rewrite the file.** `filesystem.allow` defaults to `["."]` (`src/config/mod.rs`). The read-only rule covers only `.devcroft/<name>/` (except `home` and `run`), and the Linux view binds the workspace, manifest included.
- **The file grants itself authority.** `filesystem.allow` and `read` widen access. `network.default`, `allow`, `loopback` and `ports` open routes, and `provisioning.network.allow` opens the hook's. `[env] forward` reads host values in `apply_manifest_env`. Separately, the user's `[credentials] env` and `[[credentials.broker]]` reach every sandbox the user brings up.

`maybe_auto_up` accepts any healthy keeper by sandbox name before it discovers the manifest, so an attach is not tied to a checkout either. `Meta` and the compiled profile already live in devcroft's data directory, which the baseline denies to every sandbox.

The design follows the owner’s two conditions for an untrusted mode: the workload can’t reach the authoritative manifest, and the manifest can’t grant permissions without an explicit approval bound to the checkout and the manifest’s hash. The owner accepted the untrusted default as a breaking change on 2026-11-27.

## Goals / Non-Goals

**Goals:**

- A checkout gets no authority from its manifest that the user has not approved for those exact bytes.
- The workload cannot alter the file its next generation's policy is read from.
- Starts and attachments are decided the same way, before any phase reads project input.
- What was not applied is visible in `up`, `status` and `policy --render`.

**Non-Goals:**

- Verifying code. An approval covers the manifest only, not the flake, lockfiles, scripts or provider version. A change in those can change what runs without changing the hash.
- Confining host-side provider resolution, which still evaluates repository definitions with the daemon (`sandbox-provisioning` P6). Data-only providers (`mise` with `http:` tools, `pixi` with a Conda lock) execute nothing from the repository there. That is documented, not enforced here.
- Hiding the manifest’s contents. Git history keeps them readable, and the manifest holds no secrets. The effective policy already lives in host state.
- A stronger boundary. The native backend still shares the kernel; microVM is a separate direction.

## Decisions

**D1. One read, one hash.** `config::load` opens the manifest with `O_NOFOLLOW`, requires a regular file with one link, reads it once, and returns the parsed manifest together with the SHA-256 of those bytes. A symlinked or multiply linked manifest fails at layer `config`. Alternative rejected: hashing in one read and parsing in another leaves a window where the approved bytes and the applied bytes differ.

**D2. Checkout identity.** An approval is keyed by the canonical project root and the root directory’s `(st_dev, st_ino)`. Another worktree, a moved checkout or a re-clone at the same path is a new checkout. Alternative rejected: the Git remote or the sandbox name. Either is a hint that anyone can copy, not an authorization.

**D3. Approval store.** One record per checkout lives under `<data_dir>/trust/`, holding:

- the canonical root;
- the directory identity;
- the approved hash;
- the time of approval;
- the state (`approved`, `invalidated` or `revoked`);
- the grants that the approval made effective.

The data directory is baseline-denied to every sandbox, so no workload can forge or read a record.

**D4. Invalidation is sticky.** When a start or an attach sees a hash other than the approved one, the record becomes `invalidated` and stays so. Returning to the old bytes does not revive it, and only `devcroft trust` approves again. This keeps a one-byte change by any source from being silently undone later. Formatting changes count.

**D5. One effective manifest.** Admission turns the loaded manifest into an `EffectiveManifest`, and every later phase reads only that type: preflight, the proxy decision, provider resolution, provisioning, environment capture, compilation, rendering and `Meta`. When the checkout is untrusted, admission clears each widening key and records it in a `NotApplied` list (key and summary, never a value):

- `filesystem.allow` beyond the workspace, and `filesystem.read`;
- `network.default = "allow"`, `network.allow`, `network.loopback = "any"`, `network.ports` and service ports;
- `provisioning.network.allow`;
- `env.forward`;
- `ssh.forward_agent`.

Each of these widens what the sandbox can reach or what can reach it:
- `network.ports` and service ports make a sandboxed listener reachable from the host, where a browser or a local tool talks to it;
- `ssh.forward_agent` bridges the user's SSH agent socket in.

Neither is something a repository may grant itself. A filesystem grant whose path is absent at `up` goes on the same list, with the reason `absent`. Today it's dropped silently while `policy --render` still shows it granted (`docs/known-gaps.md`).

Narrowing keys stay applied: `filesystem.deny`, `network.default = "deny"` and `require_native_apple_evidence`. `[env.vars]` is literal data and stays applied. When clearing leaves an invalid combination, `up` refuses at layer `config` naming `devcroft trust`.

The alternative was gating in the keeper. It would not cover the phases that run host-side before restriction (`resolve_for_up`, `__provision`). Making the raw grants unreachable after admission is the structural guard.

**D6. The workspace grant is a base rule.** The project root’s read-write grant gets origin `workspace` and is compiled in both states. The parser’s default `allow = ["."]` stops being the source of it. A manifest that writes `"."` explicitly adds nothing.

**D7. The user’s credentials follow trust.** `[credentials] env` and `[[credentials.broker]]` from the user configuration are delivered only to trusted checkouts, and `up` names the ones it withheld. A selected agent recipe’s credential is an explicit per-sandbox request (`add-agent-packages`), so it is still delivered.

**D8. Admission at attach.** `Meta` records each generation’s trust state, manifest hash and approval. An attach is allowed when the generation is `untrusted`, or when the current bytes hash to the generation’s recorded hash and the record is `approved` for it. Otherwise the attach fails at layer `config` with exit code 2. The message names `devcroft down` followed by `up` (which starts untrusted), or `devcroft trust` followed by `up --recreate`. Every attach first resolves the checkout and refuses a sandbox whose recorded root differs, naming `--name`. Every refusal goes to `stderr` only. `proxy` writes nothing to `stdout` when it refuses, because its `stdout` belongs to the SSH stream. That replaces `maybe_auto_up`’s lookup by name alone. Sessions already running are not ended, and `status` says that a changed hash does not revoke them.

**D9. Protecting the file.** This applies in both states.

- **Linux:** the view binds a devcroft-owned empty file with mode `0000`, read-only, over `<root>/devcroft.toml`. An open fails with `EACCES`. Writing, removing or renaming fails, because the path is a read-only mount point. The name stays listed, and `status` says it is masked rather than absent.
- **macOS:** Seatbelt denies `file-read*` and `file-write*` on the manifest’s path.
- **Both:** measure the alternative names for the same file: case variants on a case-insensitive volume, the `/private` prefix, symlinks inside the workspace and hard links made inside the sandbox. An alias that stays open makes protection unavailable on that host, and a host where protection is unavailable refuses `up` rather than starting without it. Anything inside the sandbox that read the manifest gets the effective configuration instead.

**D11. Git state of an untrusted checkout.** Protect `.git/config`, `.git/config.worktree` and `.git/hooks/` read-only, with the D9 mechanism. Git runs them on the host without review:
- hooks;
- `core.fsmonitor` and `core.hooksPath`;
- the commands that filter drivers, `core.sshCommand` and `core.pager` name.

So a planted entry is host code execution the next time the user runs `git`. Objects, refs and the index stay writable, so an agent can still commit. A trusted checkout keeps these writable, and `docs/known-gaps.md` records that.

Alternative rejected: making all of `.git` read-only, as `add-file-only-workflow` does. Where no project command runs, nothing needs to commit; in a managed sandbox, an agent does.

**D12. Providers are qualified for untrusted checkouts one by one.** Each provider’s host-side phases (resolution, capture, provisioning of the activation script) are run against the acceptance cases with sentinel values. A provider that reads an unapplied key, for example a host variable through its own environment capture, refuses untrusted checkouts until it is fixed. A provider not yet qualified refuses them too, so the default fails closed.

**D10. The approval prompt.** `devcroft trust` prints the checkout, the hash and the grants that would become effective, and, when an earlier record exists, what changed since it. On a TTY it asks `y/N`. Otherwise it needs `--yes --sha256 <hex>` and refuses a hash that does not match the bytes, so a script cannot approve whatever happens to be on disk. `init` records an approval for exactly the bytes it wrote, and says so.

## Risks / Trade-offs

- [Approval fatigue] → Show only the effective grants and what changed since the last approval, never the whole file.
- [`trusted` read as “reviewed code”] → `trust` and `status` say what the hash covers, and that provider resolution evaluates repository definitions host-side.
- [Editing the manifest makes attach refuse until recreation] → Accepted. The refusal names both remedies, and a running generation keeps its sessions.
- [macOS alias not closable] → The measurement decides. Refusing is the required outcome rather than a silent gap.
- [Every sample and test changes] → Accepted: no users. Test helpers approve by hash, and each sample’s README gains the `trust` step.
- [A provider stays unqualified for untrusted checkouts] → That provider’s untrusted checkouts refuse with a named reason; approving the manifest is the remedy.

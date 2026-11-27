## Why

A repository's `devcroft.toml` decides what its own sandbox may reach: extra filesystem grants, egress, published ports and forwarded host variables. Today devcroft applies it as written. The workload can also rewrite it, because the default `filesystem.allow = ["."]` grants that file read-write, so the next `up` applies whatever sandboxed code wrote. Running agents on repositories the user did not write is the 1.0 goal, and it needs the opposite default: the repository asks, and the user approves.

## What Changes

- **BREAKING** Every checkout's manifest is `untrusted` until the user approves it. An untrusted manifest still selects the provider, services, hooks and identity, and still narrows the policy. It grants nothing beyond the workspace, the provider and the baseline:
  - no extra filesystem grants;
  - no egress, loopback widening, published ports or provisioning network;
  - no `[env] forward`;
  - the user's own `[credentials]` are not delivered.

  `up` names every key it did not apply. devcroft has no users yet, so no migration path is offered (owner decision, 2026-11-27).
- `devcroft trust` approves the exact manifest bytes for one checkout after showing the grants that become effective. Non-interactively it needs `--yes --sha256 <hex>`. `devcroft trust --revoke` withdraws the approval. `init` approves the bytes it writes.
- The approval is bound to the checkout’s canonical root and directory identity, and to the SHA-256 of the bytes devcroft parses, read once. A different hash invalidates the approval permanently. Neither `--recreate` nor auto-up re-approves.
- Admission covers attachment. Every path that attaches to a generation started `trusted` re-checks the hash and refuses when it no longer matches: `AlreadyUp`, auto-up in `exec`/`shell`/`ssh`/`proxy`, `agent run` and `agent acp`. Every attach also checks that the sandbox belongs to the discovered checkout.
- The workload cannot read, modify, delete or replace the authoritative manifest, even with a read-write workspace. A symlinked or multiply linked manifest is refused. `status` reports whether the name is still listed.
- The workspace grant becomes a base rule with origin `workspace`, separate from the parser’s default `filesystem.allow = ["."]`. `policy --render` shows unapplied manifest keys as not applied.

## Capabilities

### New Capabilities

- `manifest-trust`: the untrusted default, approval and its binding, invalidation, admission of starts and attachments, phase ordering, and delivery of the user’s credentials.

### Modified Capabilities

- `policy`: the `workspace` base rule, rendering of unapplied keys, and protection of the manifest from the workload.
- `cli`: the `trust` command, trust state in `status`, and approval by `init`.

## Impact

- Code: `config::load` (bytes and hash from one read), the start and attach paths in `lifecycle` and `maybe_auto_up`, policy compilation and rendering, the Linux mount view (`fleet::mount::construct_view`), Seatbelt platform rules, delivery of user credentials, `init`, and `USAGE` with `tests/cli_help_and_version.rs`.
- Every sample and test whose manifest grants anything beyond the workspace must be approved first. Test helpers approve by hash.
- Fleet applies the same decision.
- [Agent packages](../add-agent-packages/proposal.md) are unaffected: recipes, hosts and credentials come from the user configuration and an explicit per-sandbox selection. The [file-only workflow](../add-file-only-workflow/proposal.md) reads no manifest, so there is nothing for it to approve.
- Unchanged and stated: provider resolution still evaluates repository definitions host-side. An approval covers the manifest’s bytes, not the code, lockfiles or flake they point to.

## Why

The owner redefined 1.0 on 2026-11-27. devcroft 1.0 shows that an agent can work in a project with or without a reproducible environment, takes no ambient host toolchain, and that the repository can't grant itself permissions. These changes deliver the behavior:

- `add-manifest-approval`;
- `add-file-only-workflow`;
- `add-agent-packages` and `add-agent-editor-acp`;
- `exclude-swift-from-standard-build`.

None of them covers what a release must itself prove:

- which surface is stable;
- how to report a vulnerability privately;
- that the distributed artifact installs and upgrades;
- that the supported matrix was actually verified;
- that a crash leaves nothing with authority behind.

[The roadmap](../../../docs/roadmap.md) still defines 1.0 by the fleet claim, and the README tells security reporters to open a public issue.

## What Changes

- Rewrite the roadmap’s 1.0 section to the new criterion. Fleet, microVM and Mode 2 become non-blocking for 1.0.
- Define the stable surface. It covers:
  - the CLI’s commands, flags, error layers and exit codes;
  - the `devcroft.toml` and user-configuration schemas;
  - the behaviors the docs state.

  The Rust `pub` items stay explicitly unstable, as `src/lib.rs` already says.
- Publish a private security-reporting channel in `SECURITY.md` before 1.0, and point the README at it.
- Add a release gate that verifies the support matrix. It covers each platform, provider, backend and agent combination declared stable. Any skipped mandatory probe keeps that combination out of the stable set. Linux is the reference platform, and macOS is promoted one combination at a time.
- Test the distributed artifact. Install from the published package, and upgrade from the previous release with existing sandboxes, `Meta` and configuration. Each existing sandbox either runs or is refused with `up --recreate`, never misread.
- Clean up after a crash. When a keeper, proxy or agent process dies, nothing with the sandbox’s authority keeps running. `status` reports what happened, and the next `up` recovers.
- Run two worktrees of one project in parallel, end to end, as a release check.

## Capabilities

### New Capabilities

- `release`: the stable surface, the security channel, the release gate, and install and upgrade from the distributed artifact.

### Modified Capabilities

- `lifecycle`: cleanup after a crash of a keeper, proxy or agent process.

## Impact

- **Docs:** `docs/roadmap.md`, a new `docs/stability.md`, `SECURITY.md` and the README.
- **CI:** a release workflow that reads the qualification records and the provider rows.
- **State:** `Meta` versioning for upgrades, and crash handling in `lifecycle` and `fleet::lifeline`.
- **Owner:** the security channel needs the owner’s own account, for GitHub private vulnerability reporting or a mailbox.

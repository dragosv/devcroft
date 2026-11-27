# Use the packaged agent from one editor

## Why

A developer who already has the agent working in a terminal should be able to open the same project from a GUI editor, edit a file and run its test.

## What Changes

Expose the first qualified agent’s native ACP endpoint on a sandbox that is already running, and print a working setup snippet for one editor. `agent acp` never starts a sandbox. When the sandbox is down or still being prepared, it fails at once on `stderr` with the exact command to run. Closing one conversation ends its process group and leaves project services and other conversations available.

This is part of the 1.0 release in this attach-only form (owner decision, 2026-11-27). Starting a sandbox from the editor is a later change. That change needs a single-flight preparation protocol keyed by generation, which this one does not have to define.

**Depends on** [the interactive agent](../add-agent-packages/proposal.md). Qualify one editor using the operating system’s GUI launcher.

## Capabilities

### Modified Capabilities

- `agent-packages` covers ACP attachment, transport and cleanup.
- `cli` adds editor configuration and handshake diagnostics.
- `config` adds a user-owned credential command that `up` can use instead of an exported key.

## Impact

Changes affect the host agent CLI, session cleanup and the user configuration. This delivery uses the existing native recipe.

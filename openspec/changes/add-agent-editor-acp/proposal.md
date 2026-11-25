# Use the packaged agent from one editor

## Why

A developer who already has the agent working in a terminal should be able to open the same project from a GUI editor, edit a file and run its test.

## What Changes

Expose the first qualified agent’s native ACP endpoint, print a working setup snippet for one editor, and bound startup time. Closing one conversation ends its processes and leaves project services and other conversations available.

**Depends on** [the interactive agent](../add-agent-packages/proposal.md). Qualify one editor using the operating system’s GUI launcher.

## Capabilities

### Modified Capabilities

- `agent-packages` covers ACP session startup, transport and cleanup.
- `cli` adds editor configuration and handshake diagnostics.
- `config` adds a user-owned credential source for GUI launches.

## Impact

Changes affect the host agent CLI, lifecycle lock use, session cleanup and the user configuration. This delivery uses the existing native recipe.

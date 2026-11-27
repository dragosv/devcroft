# Qualify Claude Code and its editor adapter

## Why

Claude’s terminal executable and its editor adapter have different distribution and runtime requirements. A working standalone CLI does not prove the adapter uses a compatible runtime or preserves login state.

## What Changes

Qualify the standalone Claude Code CLI and the ACP adapter paired with its SDK. Pin each component, protect the launch environment, preserve personal settings and test brokered keys and subscription login per platform.

**Depends on** [agent packages](../add-agent-packages/proposal.md) and the [editor endpoint](../add-agent-editor-acp/proposal.md) for end-to-end acceptance.

## Capabilities

### Modified Capabilities

- `agent-packages` extends installation, launch and authentication to the Claude recipe.
- `policy` adds login destinations for the selected mode.

## Impact

The change introduces the required Node and npm layouts, per-platform authentication modes, the managed Claude denial hook in the sandbox HOME (amending `add-agent-workload`’s rule that nothing writes the hook at `up`, which is meant for the project’s files), and qualification evidence for each advertised platform.

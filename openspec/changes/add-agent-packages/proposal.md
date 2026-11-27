# Run one packaged agent in the project terminal

## Why

A developer should be able to ask an agent to edit a file and run the project’s own test without installing a second toolchain on the host or changing the project’s provider.

## What Changes

Ship one qualified native executable, one named model provider and one brokered authentication method. `devcroft agent run` prepares the package and opens its terminal. Explicit installs persist. The user can see the selected version, the destinations it adds and which sandbox they are using. The agent starts only through devcroft (`agent run` here, `agent acp` in the editor change). Nothing is added to the sandbox’s `PATH`, so a project command never resolves to the agent’s runtime. A launcher on `PATH` is left to a later change.

**Release dependencies.** [Safe recreation](../validate-before-recreate/proposal.md) protects the running project during preparation. [Terminal transport](../support-osc7501-terminal/proposal.md) must also pass on `agent run`. The first release is accepted when the agent edits a file, runs the project test with the project’s tools, its `agent run` path passes bidirectional OSC 7501 transport qualification (`T01`–`T10`, `T12`), and its published capability profile records whether the harness natively emits OSC 7501 (`OSC-05`, `S9`).

## Capabilities

### New Capabilities

- `agent-packages` covers verified packages, persistent selection, installation and interactive sessions.

### Modified Capabilities

- `cli`, `config`, `credentials`, `policy` and `lifecycle` expose and enforce the selected recipe.

## Impact

The first candidate is OpenCode, with Goose as a fallback subject to the same measurements. [Editor access](../add-agent-editor-acp/proposal.md) builds on this change. [Claude packaging](../add-claude-agent-recipe/proposal.md) and the [isolation experiment](../add-agent-executor/proposal.md) have their own acceptance criteria.

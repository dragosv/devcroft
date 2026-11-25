# Test whether project commands can run separately from the agent

## Why

A developer may want the project’s tests to use its own tools without being able to read the agent’s credential. The shared sandbox used by the first agent cannot promise that separation.

## What Changes

Run a bounded experiment with a fixture and one qualified real agent. Record where its test command runs, who can read a synthetic credential and which workspace settings influence the agent. The deliverable is evidence and a decision about a future product change.

**Depends on** [one qualified agent](../add-agent-packages/proposal.md). It can proceed independently of Claude and the editor delivery.

## Capabilities

### New Capabilities

- `project-executor` defines acceptance of the experiment and its decision record.

## Impact

Use an isolated test workspace and disposable credentials. Public mode selection, migration, additional proxies in production and platform rollout require a subsequent proposal.

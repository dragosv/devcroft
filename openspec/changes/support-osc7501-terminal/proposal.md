# Carry agent status through interactive terminals

## Why

An agent can report its state to the developer’s terminal. The exchange needs a reply in the same session, even when that reply ends without Enter.

## What Changes

Qualify OSC 7501 on the existing shell and SSH paths. Keep sessions independent, restore the terminal when the session ends, and prevent a log viewer from replaying old status reports as live updates. This can ship alongside `validate-before-recreate`, before agent packages, except task 2.3: its live emitter is driven by `fake-llm` from `add-agent-packages`.

## Capabilities

### Modified Capabilities

- `exec` covers interactive terminal transport and restoration.
- `ssh` covers the same exchange on an SSH session with a PTY.
- `cli` covers how `devcroft logs` shows recorded output on a terminal.

## Impact

The exec client, keeper, SSH relay and log viewer are in scope. `add-agent-packages` qualifies the new `agent run` entrypoint against this contract.

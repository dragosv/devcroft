# Keep a working sandbox through failed preflight

## Why

Recreating a healthy sandbox can stop its keeper before discovering that a required credential is missing. The developer then loses a working session for an error that was checkable at the start.

## What Changes

Run the existing credential and broker checks before teardown or provider resolution. Fix `rm`’s critical section too: it reads `Meta` before taking the lifecycle lock and sweeps shared objects after releasing it, which every store provider, agent objects and the pinned runtime depend on. An ordinary start of an already healthy sandbox still works from a shell without those credentials. This can ship independently of agent recipes and terminal support.

## Capabilities

### Modified Capabilities

- `lifecycle` governs explicit and automatic startup.

## Impact

The startup order changes in `lifecycle::up` and the CLI auto-up path. Future agent downloads use the same pre-teardown boundary in `add-agent-packages`.

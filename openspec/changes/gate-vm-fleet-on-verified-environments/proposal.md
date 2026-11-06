## Why

A working local disk, lockfile or image digest does not demonstrate that a fresh
agent can reconstruct the project's environment. Fleet needs evidence tied to
the exact definition and source context it will launch, without inheriting
interactive state or another agent's credentials.

## What Changes

- Require an explicit reconstructible definition, pinned recoverable inputs,
  declared setup/services and a non-empty verification workload for VM fleet.
- Reconstruct in a separate clean VM using only declared inputs and eligible
  immutable caches; preserve the exploratory local instance.
- Bind host-owned evidence to environment, source context, policy, tooling,
  runtime compatibility and verification identity; invalidate changed inputs.
- Recheck eligibility at launch and gate every agent on its own setup/readiness
  for its exact source. Share immutable bases, never mutable service data.
- Keep snapshot continuity, filesystem identity and verified reconstruction as
  distinct claims. A digest-only image is local-usable but insufficient for
  this gate, as confirmed by the user on 2026-11-06.
- **BREAKING for VM fleet:** an incomplete or stale environment is refused
  before launching agent work. Existing process fleet is not relabeled or
  silently migrated by this proposal.

## Capabilities

### New Capabilities

- `verified-fleet-environment`: reconstruction evidence, identity and staleness,
  transactional admission and per-agent startup checks.

### Modified Capabilities

None for the current native fleet. The existing `agent-supervisor` and
`workspace-isolation` plans must be reconciled when adding VM dispatch; their
shared host Git mirror is not inherited as a VM authorization mechanism.

## Impact

Definition storage, preparation controller, artifact cache and fleet admission,
status and diagnostics. Depends on `add-vm-project-runtime` and
`add-private-local-environments`. No remote scheduler, universal reproducible
build proof, credential broker, or automatic recipe inference is introduced.

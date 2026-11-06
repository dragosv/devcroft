## Why

A project without a complete provider definition should be useful locally
before it is eligible for fleet. A private guest workspace makes that possible
without giving unfamiliar code write access to the original checkout.

## What Changes

- Add deterministic environment selection for the VM path: explicit selection,
  explicit external binding, project definition, unambiguous detection, then a
  named devcroft base. Report ambiguity instead of guessing a complete setup.
- Import an explicitly described source snapshot into private guest storage, with
  untracked files left out by default; refresh it on request and offer staged,
  advancing export with conflict checks and no automatic host execution.
- Permit local package installation and persistent adaptations in the private
  VM through explicit setup windows, with no writes to shared bases.
- Save an editable environment definition inside or outside the repository;
  never claim to infer a complete recipe from shell history or disk changes.
- Support a bounded Dev Containers subset: digest-pinned image, effective image
  metadata and supported lifecycle commands in the guest. Refuse Dockerfile,
  Features, Compose and host-side initialization by name in this phase. Classify
  every specified field as honored, ignored with a warning or refused, so that
  official images pass.
- **BREAKING for the VM path:** lack of a provider no longer implies failure,
  working files are in the guest rather than a writable host checkout, and `rm`
  can now remove the only copy of unexported work.

## Capabilities

### New Capabilities

- `private-project-workspace`: bounded import, guest editing, staged export and
  preservation of host data.
- `local-environment-preparation`: selection, adaptable bases, external
  definitions, bounded Dev Containers compatibility and honest status.

### Modified Capabilities

None in the native path. This explicitly replaces the VM-path assumptions of
`add-manifestless-mode` (no default base and automatic precedence) and does not
silently extend `add-devcontainer-provider`'s Linux process-tier rootfs design.
See the design for the reconciliation required before either ships as VM UX.

## Impact

Config/CLI, workspace transfer, provider adapters, guest setup and lifecycle.
Depends on `add-vm-project-runtime`. Image-only local support is not fleet
eligibility; the strict reconstruction gate is specified separately.
Public command spelling and the shipped base catalog are release decisions,
not reasons to assume arbitrary host tools or arbitrary default packages. How a
base installs system packages is still open; whatever is chosen must let a recipe
pin them by content hash, or that base stays local.

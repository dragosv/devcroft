## Why

`devcroft up --vm` runs a different product from native `up`. Native takes the
toolchain, the shell and the userland from the project's provider lock and
records where each comes from. The VM boots a fixed bundle image, ignores
`env.provider` except to refuse Swift, and gives the guest whatever that image
holds. An adversarial review of the `microvm` branch on 2026-11-28 also found
that the VM path renders authority it does not enforce, lets a project manifest
forward host secrets into the guest, and loses work through a copied workspace.

The `microvm` branch does not merge until VM mode offers what native offers for
the same `devcroft.toml`, and until each review finding is either fixed or
recorded as a decided limit.

## What Changes

- The guest's environment comes from the project's provider, resolved for the
  Linux target inside a preparation VM, as `add-vm-project-runtime` R3 already
  requires. A fixed bundle image is no longer the project's toolchain.
- The guest's shell and base userland follow the same resolution order and
  provenance rules as native (`docs/base-runtime.md`): provider first, then the
  closure, then devcroft's runtime at a recorded generation. `up` and `status`
  report the same provenance record for both runtimes.
- A provider or lock that cannot be resolved for the Linux target is refused by
  name, never replaced by the bundle's tools.
- `policy --render` shows only what the VM is given. The rendered origins come
  from real user grants; a project request with no grant renders as a request,
  not as granted.
- `[env] forward` and other project-controlled keys never move host values into
  a VM without a user-owned grant, and every value that does reach the guest is
  named at `up`.
- `sync-out` cannot write files the host would later execute or that change
  devcroft's own configuration.
- The workspace model reaches native parity or states its difference: deletions,
  `.git`, size limits, conflicts, data loss at `rm`/`--recreate`, and the working
  directory of `exec`/`shell`.
- The CLI surface for VM mode is discoverable from `USAGE`, consistently named,
  checked by `doctor`, and described in README, known gaps and the threat model.
- `add-vm-project-runtime`'s task evidence is corrected to say what the CLI path
  runs, and tasks whose behavior the CLI does not reach are reopened.
- The hvi pending-bound evidence includes the 2026-11-23 stop runs it omitted.

## Capabilities

### New Capabilities

- `vm-native-parity`: the same `devcroft.toml` yields the same provider
  environment, shell, base userland, provenance record, working directory and
  lifecycle guarantees in VM mode as natively, or a named refusal.
- `vm-authority-enforcement`: the VM's rendered authority equals its enforced
  authority, project-controlled keys cannot move host secrets or host-executed
  files across the boundary, and refusals name the missing grant.
- `vm-cli-surface`: VM commands and flags are listed, named consistently,
  diagnosed by `doctor`, and documented with their limits.

### Modified Capabilities

None. `openspec/specs/` holds no main specs yet. This change constrains the
unarchived `vm-project-runtime` and `vm-authority-boundary` capabilities of
`add-vm-project-runtime`, and is a merge gate for the `microvm` branch.

## Impact

- `src/vm/cli_runtime.rs`, `src/vm/preparation.rs`, `src/vm/authority.rs`,
  `src/vm/proxy_channel.rs`, `src/vm/credentials.rs`, `src/bin/devcroft.rs`.
- Guest image contents: devcroft's runtime (uutils and brush) built for
  `linux-arm64`, and the preparation image holding pinned provider tools.
- `docs/base-runtime.md`, README, `docs/known-gaps.md`, `docs/threat-model.md`,
  `AGENTS.md`, `USAGE`.
- `openspec/changes/add-vm-project-runtime/tasks.md` (reopened tasks) and
  `experiments/microvm/results/macos-arm64-hvi-pending-bound.json`.
- A workspace decision may revisit the 2026-11-26 owner decision recorded in
  `docs/decisions.md` (private ext4 disks, no `virtio-fs`).

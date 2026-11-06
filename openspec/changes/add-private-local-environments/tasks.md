## 1. Define the local interface

- [ ] 1.1 Confirm runtime qualification; select the versioned initial base catalog, its hash-pinnable package mechanism, the exploration profile's hosts and concrete CLI syntax for selection/save/export/refresh.
- [ ] 1.2 Define the versioned recipe and explicit external binding schema, separating environment requests from user grants.
- [ ] 1.3 Implement deterministic selection and non-TTY ambiguity failures; test missing, invalid and competing definitions.

## 2. Transfer private workspaces

- [ ] 2.1 Implement bounded snapshot inventory with dirty/untracked/excluded reporting, untracked files out by default, user global excludes, the sensitive-path list and concurrent-change detection.
- [ ] 2.2 Implement isolated Git interpretation and controlled guest metadata, with HEAD-reachable history by default, remote URLs without userinfo and explicit refusal of unsupported layouts.
- [ ] 2.3 Test traversal, symlinks, alternates, hooks, filters, special files and outside-root access with host sentinel controls.
- [ ] 2.4 Implement staged export: commit series through a confined worker, file changes including binaries/deletions/modes, base identity, advancing export points and host collision checks.
- [ ] 2.5 Test export cannot alter host checkout or Git metadata, including malicious paths and changed host source.
- [ ] 2.6 Implement explicit refresh with guest-side merge, conflict reporting and credential drop.
- [ ] 2.7 Name unexported work in `rm` and `up --recreate` confirmations; test guest-reported and unknown counts.

## 3. Adapt and save local environments

- [ ] 3.1 Add private administrative setup through setup windows under user scope and preserve workspace/adaptations/service data across restart.
- [ ] 3.2 Add explicit draft save to repository or external location, preserving omissions and excluding secret values.
- [ ] 3.3 Test project rebinding, same-basename checkouts, changed origins/targets and external definition revisions.
- [ ] 3.4 Expose guest workspace, the editor path for the project's trust class, health, setup state and unverified/ineligible local status independently.

## 4. Add the bounded image profile

- [ ] 4.1 Pin the Dev Containers specification/schema used for supported fields and metadata merging; classify every key and publish the honored/ignored/refused table.
- [ ] 4.2 Resolve platform-specific image digests, record tag resolution and verify/extract layers inside isolated preparation with private writable runtime state; refuse a missing architecture and private registries by name.
- [ ] 4.3 Implement effective metadata merging, supported guest users/environment, one warning for ignored fields and named refusal of fields needing host authority or an execution the profile does not perform.
- [ ] 4.4 Implement supported lifecycle order and durable phase results, mapping creation commands to setup and `postStartCommand` to runtime policy, including explicit retry after partial failure.
- [ ] 4.5 Test metadata-smuggled privileges/mounts, localEnv leakage, Dockerfile/Features/Compose refusal and unknown keys; `devcontainers/rust:1` passes with warnings.
- [ ] 4.6 Run a complete local image project and an unconfigured project through edit, restart, export and external draft save.
- [ ] 4.7 Reconcile manifestless/provider plans for VM scope and document that image-only local success is not strict fleet eligibility.
- [ ] 4.8 Measure the profile against real `devcontainer.json` files and official images, and publish the result.

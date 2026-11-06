## Context

The owner confirmed an initial Dev Containers scope of pinned images plus guest
lifecycle, with Dockerfile, Features and Compose separate. Local usefulness must
not depend on fleet eligibility. The VM runtime supplies isolation; this change
supplies project selection, private working state and a route to a saved recipe.

## Goals / Non-Goals

**Goals:** open an existing project, adapt a known Linux base, edit remotely,
export deliberate changes and save a definition without requiring upstream adoption.

**Non-Goals:** complete automatic environment inference, host checkout sharing,
full Dev Containers compatibility, direct URL cloning or snapshot promotion.

## Decisions

### L1 — Selection is deterministic without concealing ambiguity

For the new VM entry path: explicit source selection, explicit user-owned external
binding, project devcroft definition, unique supported detected source, named
default devcroft base. Detection is bounded data inspection in the selected
project root, never provider activation. Competing detected sources require an
explicit choice; interactive use can ask, non-TTY returns candidates and exits.
An explicit invalid source fails rather than falling through to a generic base.

The fallback is a versioned product base with a fixed identity and Linux target,
not host passthrough or a claim of complete tooling. Display its origin and missing
project setup. Language manifests may suggest a profile, never silently select a
different toolchain or count as a complete definition. Choose the actual base
catalog in the packaging task; no guessed package versions are specced here.

### L2 — Private workspace is the first and only workspace mode in this change

Import a content-identified snapshot into `/workspace`. For ordinary Git projects
include tracked working-tree contents (including local edits/deletions) and
non-ignored untracked files; exclude ignored files unless explicitly selected.
Report included/excluded paths and that staging/index state is not preserved as
an active index. Never omit dirty state silently or represent a HEAD-only clone
as the current checkout. Concurrent host changes fail snapshot consistency or
require a new snapshot; they do not become mixed-version evidence.

Host transfer reads only selected bounded regular files and symlink entries,
without following links outside the selected root. Reject device nodes, sockets,
path traversal and unsupported metadata. Do not execute host Git against project
config. Interpret Git objects/refs/ignore rules in a confined import worker;
rebuild guest administrative metadata with controlled config, no hooks, no host
alternates, no credential helpers or external filters. Copying inert object data
for isolated interpretation is different from activating the original `.git`.

Initially reject linked worktrees, submodules, external alternates and unsupported
Git layouts with actionable diagnostics unless an explicitly selected plain-file
snapshot avoids the unsupported history semantics. Monorepo subproject roots
are explicit; parent configs are not discovered automatically. Non-Git roots use
an explicit file snapshot and exclusions, with no invented Git history.

Export first produces a staged patch/file bundle outside the original checkout,
including additions, deletions, binary contents and mode changes plus the import
base identity. Validate paths, sizes, symlink targets, `.git` administrative paths
and case/normalization collisions for the host filesystem. No export command
runs hooks, applies changes or invokes Git on guest metadata on the host.
An optional later apply operation requires an explicit request and a base/conflict
check; initial implementation delivers the staged result for review. Exported
project scripts remain code; staging is not a declaration that they are safe.

### L3 — Adaptation is private and persistence is explicit

Local setup may install system dependencies with guest administrative authority
scoped by the user, while ordinary sessions use their declared guest identity.
This cannot change shared bases, host packages or siblings. `down` and restart
preserve local adaptations, workspace and service data; clean rebuild creates a
different instance. Status does not pretend to identify every undeclared disk
change: local state is unverified until a separate definition is reconstructed.

### L4 — Save a recipe, not a transcript

A versioned definition records environment source, base/target, pinned inputs,
setup order, env values or secret references, services/readiness/seed data,
phase network requests and verification commands. An explicit save writes a
draft to a chosen repository path or user-owned external path. The draft can be
incomplete and must say so; generated suggestions do not imply correctness.

External binding uses a controller-generated project ID associated explicitly
with canonical root, origin identity when available, subdirectory and target.
The definition revision/content digest participates in preparation identity.
Directory basename alone is never a key; moving a checkout or changing its
origin requires explicit rebinding. Definitions are portable requests, while
host grants remain separate user-owned policy. No automatic search loads someone
else's binding merely because they used the same repository name.

### L5 — Dev Containers compatibility is a field-level contract

Resolve a platform-specific image digest, verify layers and prepare the rootfs
inside the preparation VM. Use an immutable base plus private writable state.
Read relevant `devcontainer.metadata` as well as the project JSON; implement and
test merge behavior from the pinned upstream specification before claiming
compatibility. Metadata cannot smuggle an otherwise refused requirement.

Initial supported operative fields: `image`, `containerEnv`, `remoteEnv`, guest
`containerUser`/`remoteUser`, and `onCreateCommand`, `updateContentCommand`,
`postCreateCommand`, `postStartCommand`. Honor supported lifecycle ordering:
creation phases once per successful creation, start phase on each start; failures
are recorded, never marked complete, and explicit retries disclose possible
partial side effects. Guest users must resolve; privileges require the scoped
setup/runtime grant and cannot confer host authority. Environment substitutions
are limited to documented guest values and explicit user-owned bindings; arbitrary
host `${localEnv:...}` access is refused.

Accept display-only `name` and editor `customizations` as non-executed metadata
with clear status. `workspaceFolder` is accepted only when matching `/workspace`.
Reject `build`, `dockerFile`, `features`, Compose configuration, `initializeCommand`,
`postAttachCommand`, `mounts`, `workspaceMount`, `runArgs`, `privileged`, extra
capabilities/devices, automatic port publication and unimplemented lifecycle or
environment options by field path. Unknown operative fields fail closed. For
port access use explicit SSH local forwarding; do not silently ignore requested
`forwardPorts`. Unsupported shell-based environment probes are refused.

This deliberately narrow profile may reject many popular images/configurations.
Publish fixtures for its supported fields and refusals instead of advertising
full Dev Containers support. Image-only local operation cannot pass the strict
fleet gate without a supported reconstructible environment recipe; this phase
does not build Dockerfiles even when a recipe is supplied.

## Risks / Trade-offs

- Private workspace adds transfer/export friction → report location and Remote-SSH
  endpoint prominently, preserve local work and stage export without overwrites.
- Dirty/untracked files may contain sensitive data → show import scope and allow
  explicit exclusions; never import ignored files or outside-root targets implicitly.
- Git and archive data attack parsers → bounded transfer and isolated interpretation;
  adversarial fixtures cover paths, symlinks and concurrent changes.
- Lifecycle scripts are not necessarily idempotent → durable phase results and
  explicit retry, never automatic retry of partly completed arbitrary commands.
- Small image profile excludes Dockerfile-based projects → named refusals and
  separate future build/Features/Compose changes, as chosen by the owner.

## Migration Plan

Add only the VM path after runtime qualification. Before shipping this as default
UX, reconcile `add-manifestless-mode`'s no-base/error and fixed-precedence rules,
and `add-devcontainer-provider`'s host extraction/read-only process rootfs model.
Retain their native scope and historical measurements; do not mark their tasks
complete because a separate VM implementation exists. Existing local sandboxes
and files are not moved automatically. Rollback preserves disks and export access.

## Open Questions

Choose the versioned base catalog and public CLI spelling before implementation
of help/output. Broader Git layouts and Dev Containers fields require separate
compatibility work. None permits a weaker implicit fallback in this profile.

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

How a base installs system packages is open, and the choice is constrained: a
saved recipe must be able to pin them by content hash, or that base cannot reach
fleet (`gate-vm-fleet-on-verified-environments` F1). A base declares which it is.
The catalog also owns the exploration profile's host list
(`add-vm-project-runtime` R4).

### L2 — Private workspace is the first and only workspace mode in this change

Import a content-identified snapshot into `/workspace`. For ordinary Git projects
include tracked working-tree contents, including local edits and deletions. Leave
untracked and ignored files out by default and name them with the way to select
them: a project-controlled `.gitignore` must not decide which of the user's local
files enter a VM that may be hostile, and a local `.env` is ignored only if the
repository chose to ignore it. Selected untracked files still pass the user's
global excludes, read from user config as data, and the baseline's sensitive-path
list; a sensitive path has to be selected by name. History is what HEAD reaches: no
other refs, stashes or reflogs, and remote URLs lose any userinfo. More is an
explicit choice.
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

Export first produces a staged bundle outside the original checkout: a commit
series with messages and authorship where the guest has commits, file changes
otherwise, including additions, deletions, binary contents and mode changes, plus
the base identity. A confined worker turns guest history into that series, so the
host receives patches and never guest metadata. Each export records what it
covered and the next defaults to that point, so a second export does not repeat
the first. Validate paths, sizes, symlink targets, `.git` administrative paths
and case/normalization collisions for the host filesystem. No export command
runs hooks, applies changes or invokes Git on guest metadata on the host.
An optional later apply operation requires an explicit request and a base/conflict
check; initial implementation delivers the staged result for review. Exported
project scripts remain code; staging is not a declaration that they are safe.

### L3 — Adaptation is private and persistence is explicit

Local setup installs system dependencies with guest administrative authority
through the setup window of `add-vm-project-runtime` R5, while ordinary sessions
use their declared guest identity and cannot elevate. This cannot change shared
bases, host packages or siblings. Inside the guest it is one trust domain: what a
window installs runs alongside the project's code, and no claim separates them.
A session that discovers a missing library does not gain authority; the user opens
a window, installs, and closes it. `down` and restart
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

An `image` named by tag is resolved once, and the digest is recorded in controller
state and shown; re-creation uses the recorded digest until the user re-resolves
it. An image with no variant for the guest architecture is refused by name, with
no emulation. A private registry is refused in this phase, because preparation
holds no credentials.

Initial supported operative fields: `image`, `containerEnv`, `remoteEnv`, guest
`containerUser`/`remoteUser`, and `onCreateCommand`, `updateContentCommand`,
`postCreateCommand`, `postStartCommand`. Honor supported lifecycle ordering:
creation phases once per successful creation, start phase on each start; failures
are recorded, never marked complete, and explicit retries disclose possible
partial side effects. The three creation commands run as creation-time setup.
`postStartCommand` runs under runtime policy, because a start opens no setup
window. Guest users must resolve; privileges require the scoped
setup/runtime grant and cannot confer host authority. Environment substitutions
are limited to documented guest values and explicit user-owned bindings; arbitrary
host `${localEnv:...}` access is refused.

Every key of the pinned schema has one class in a published table. Refusal is for
what needs host authority or an execution this profile does not perform; a field
that only concerns a container boundary, which does not exist here, is not a
reason to refuse an environment. `add-devcontainer-provider` D6 measured the
consequence: `mcr.microsoft.com/devcontainers/rust:1` carries `capAdd` and
`securityOpt` in its metadata, and refusing them on the merged result would refuse
every `devcontainers/*` image. That image is the acceptance fixture.

- **Honored:** the supported fields above, and `workspaceFolder` when it matches
  `/workspace`.
- **Ignored with one warning naming the field:** display-only `name`, editor
  `customizations`, `forwardPorts` and `portsAttributes` (SSH local forwarding
  covers them, as `docs/decisions.md` §3 says), `capAdd`, `securityOpt` and `init`,
  and `overrideCommand`, `updateRemoteUserUID`, `shutdownAction` and `waitFor`.
- **Refused by field path:** `build`, `dockerFile`, `features`, Compose
  configuration, `initializeCommand`, `postAttachCommand`, `mounts`,
  `workspaceMount`, `runArgs`, `privileged`, devices, and unsupported shell-based
  environment probes.

Feature `id` entries in image metadata record what was baked in; they are not
requests. Keys outside the pinned schema are ignored with one warning, because the
file is shared with other tools, as D6 decided.

This deliberately narrow profile still rejects many configurations. Measure it
against real `devcontainer.json` files and official images, and publish that
result with fixtures for its fields and refusals, instead of advertising full Dev
Containers support. Image-only local operation cannot pass the strict
fleet gate without a supported reconstructible environment recipe; this phase
does not build Dockerfiles even when a recipe is supplied.

### L6 — The workspace stays in step with the host on request

An explicit refresh imports a new host snapshot beside the workspace and merges it
in the guest, leaving conflicts there for the user and adaptations untouched. It
never runs host Git on guest data. It drops direct credentials until they are
restated, because it brings in source they were not stated for
(`add-vm-project-runtime` R4).

The definition that prepared an instance is recorded by digest in controller
state. Editing a definition in the guest changes neither the instance's
environment nor any evidence: export or save is the way out, and a new preparation
is the way back in.

The guest workspace can be the only copy of unexported work, which is new: under
the native path `rm` never touched source. `rm` and `up --recreate` say so in their
confirmation, with counts labelled guest-reported when the guest is up and
"unknown" when it is not, since the host does not mount guest disks. A guest can
under-report, which harms only its own work. `--yes` stays the non-interactive form.

## Risks / Trade-offs

- Private workspace adds transfer/export friction → report location and the editor
  path for the project's trust class prominently, preserve local work and stage
  export without overwrites.
- Leaving untracked files out can omit something a build needs → the import report
  names them and a selection adds them. An omission fails visibly; a leak does not.
- Dirty tracked files may contain sensitive data → show import scope and allow
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
and `add-devcontainer-provider`'s host extraction/read-only process rootfs model
and its D6 table, where the process tier and the VM may answer differently.
Retain their native scope and historical measurements; do not mark their tasks
complete because a separate VM implementation exists. Existing local sandboxes
and files are not moved automatically. Rollback preserves disks and export access.

## Open Questions

Choose the versioned base catalog and public CLI spelling before implementation
of help/output. The base's package mechanism is undecided: it must satisfy
hash-pinned inputs, and the fixture in `evaluate-microvm-runtimes` Stage C does not
decide it. Broader Git layouts and Dev Containers fields require separate
compatibility work. None permits a weaker implicit fallback in this profile.

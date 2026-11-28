## Context

`add-vm-project-runtime` designed a VM path whose environment is prepared from
the project's provider inside a preparation VM (R3) and whose authority is the
project's requests intersected with user grants (R4). The `microvm` branch
implements the pieces as library modules (`preparation.rs`, `publication.rs`,
`authority.rs`, `proxy_channel.rs`, `credentials.rs`), but the CLI path,
`up_vm_instance` in `src/vm/cli_runtime.rs`, does not call them. It boots a
trusted bundle, attaches the bundle's fixed workload image as the toolchain,
copies the project in with `runtime sync-in`, and starts the host egress proxy
with the manifest's `network.allow`.

Native resolves the environment once at `up` from the provider's lock, picks the
shell in a fixed order (environment `PATH` inside a grant, closure requisites,
devcroft's runtime `sh`), resolves the base userland provider-first with
devcroft's uutils/brush runtime as the floor, and records per-name provenance
(`docs/base-runtime.md`).

The owner decided on 2026-11-28 that the branch does not merge until VM mode
offers what native offers for the same `devcroft.toml`. The adversarial review of
the same day is the checklist; each finding maps to a requirement in this
change's specs.

## Goals / Non-Goals

**Goals:**

- One `devcroft.toml` gives the same provider environment, shell, base userland
  and provenance record natively and in a VM, differing only in target system.
- What `policy --render` shows for a VM is what the proxy, helper and guest get.
- No project-controlled key moves host secrets in, or host-executed files out.
- The workspace either matches native visibility or states its difference, and
  never loses work silently.
- A user can find, start, diagnose and understand VM mode from `USAGE`, `doctor`
  and the docs.

**Non-Goals:**

- Running Darwin providers (`swift`) in a Linux guest. They stay native-only.
- Native Linux or x86-64 rows. They stay unsupported per the 2026-11-26 decision.
- Hostile-project claims. VM mode stays opt-in and accident-protection framed
  until `add-vm-project-runtime` R7 ships.
- Changing native behavior, except where shared code is extracted.

## Decisions

### D1 — One environment resolver, run for the guest target

Native capture (`provider::*`, `capture::store_grants`, `src/shell.rs`, base-name
provenance) becomes target-parameterized code that runs inside the preparation
VM for `aarch64-linux`. Its output is the same captured profile native records,
plus an immutable disk published through `publication.rs` under the project's
namespace. For the closure-tier providers (`nix`, `flox`, `devbox`, `devenv`) the
disk holds the closure, mounted read-only at the same store path native uses. For
the artifact-tier providers (`mise`, `pixi`) it holds the provider's install for
the Linux target, mounted read-only where the captured profile expects it, and
the instance keeps the `artifact` guarantee label. The tooling layer resolves the
same way and is composed after the project's environment. `swift` stays
native-only.

Alternative: keep the bundle's workload image and document it. Rejected: it
breaks the product's core promise and the owner's merge gate.

Alternative: resolve on the host and copy the closure in. Rejected: R3 keeps
provider evaluation inside the boundary, and host resolution would need host
Nix tooling the VM path promised not to require.

### D2 — devcroft's runtime ships in the guest at the host's generation

The guest image carries a `linux-arm64` build of devcroft's multi-call runtime
(uutils and brush) at the generation the host binary records. The guest keeper is
already built from `src/keeper`; the runtime links into the same guest binary.
Shell and userland resolution in the guest then use the native order unchanged.
The guest image's own base (busybox in the experiment) is not on a session's
`PATH` and is not a resolution candidate.

Alternative: let the provider supply everything. Rejected: native guarantees a
floor for environments without a shell (mise, pixi), and parity needs the same
floor.

### D3 — Authority is compiled once and consumed everywhere

`up --vm` compiles `CompiledVmAuthority` from the project's requests, user grants
from `RuntimeGrantStore` and the exploration profile. The proxy allowlist,
broker routes, delivered environment values and launch record are derived from
that one value, and `policy --render` renders it. `render_vm_policy_plan` no
longer builds a grant set out of the requests. A request without a grant is
rendered as refused, and its refusal names the grant command.

User grants gain egress hosts and forwarded names, bound to project identity and
definition hash, so a changed manifest re-asks.

### D4 — Host values reach a guest only by user grant

`[env] forward` and host-referencing `[env.vars]` are requests. Without a grant
they are refused and named at `up`. Every delivered host-sourced value is listed,
not only `[credentials] env`.

### D5 — Transfers out are filtered and reviewable

`sync-out` keeps its traversal and symlink refusals and adds a refused set:
`devcroft.toml`, `.devcroft/`, `.envrc`, `.git/` (hooks and config), editor
task/launch files, build-tool configuration that executes code, and new
executable modes. Refused files go to a quarantine directory beside the
controller state for review, and are named.

### D6 — Workspace model is an owner decision

Three options, recorded rather than chosen here:

1. Share the project directory into the guest (`virtio-fs`). Native visibility,
   but it reverses the 2026-11-26 decision to use private ext4 disks without
   `virtio-fs`, and gives guest root write access to the host checkout.
2. Keep a private guest workspace and make transfer correct: a recorded base
   manifest (path, mode, content hash) from the last transfer gives three-way
   comparison, so deletions propagate, conflicts stop the transfer, `.git` is
   included, and size limits become streaming limits rather than 1,000 files.
3. Make the guest workspace the source of truth, with the host reaching it over
   SFTP or the guest-served editor, and transfer only at explicit export.

Option 2 keeps the boundary the evaluation qualified and fixes every review
finding about the copy model. Options 1 and 3 are recorded so the choice is
deliberate. Until the owner decides, the requirements apply to whichever model
ships.

### D7 — One instance lookup, not per-command branching

A single resolver returns `Instance::Native` or `Instance::Vm` for a name, and
commands match on it. `cli_runtime.rs` splits into lifecycle, authority, workspace
and rendering modules. This removes the repeated `load_vm_instance` checks in
`src/bin/devcroft.rs` and makes a new command handle both runtimes by
construction.

### D8 — CLI spelling

`--runtime <process|vm>` is the documented flag, using the docs' term for the
native path. `--vm` stays as a listed alias. `native` is accepted as an alias of
`process` during the opt-in period. Commands regroup by purpose: `runtime grant
…` for grants, `runtime bundle install` for artifacts, `workspace push|pull` for
transfer, `runtime export-disk` for export. The help test checks subcommands and
flags.

## Risks / Trade-offs

- [Preparation VMs add cold-start time] → reuse published closures per project
  and lock identity; measure against the accepted 600-second threshold.
- [Provider requalification for Linux exposes locks that never covered it] → the
  refusal names the missing target, and the docs say how to add it per provider.
- [Grants make first use slower] → one grant command accepts the current request
  set for a project and definition, shown before it is accepted.
- [The quarantine set misses a host-executed file type] → the set lives in one
  table with a test per entry; additions are cheap; the threat model names the
  residual.
- [Three-way transfer is new code on a trust boundary] → bounded manifest
  parsing, negative tests for traversal, conflicts and oversize input.

## Migration Plan

Existing VM instances were created with the bundle workload. On the first `up
--vm` after this change, such an instance is refused with a message telling the
user to transfer work out, then `up --vm --recreate`. Native instances are
untouched. `add-vm-project-runtime` tasks whose behavior the CLI does not reach
are reopened in the same change that lands this one. Rollback keeps
`DEVCROFT_VM_ROLLBACK`, now documented.

## Open Questions

- Workspace model (D6): option 1, 2 or 3.
- Whether the opt-in preview accepts unreviewed `network.allow` hosts with a
  printed warning, or requires a grant from the first run.
- Which providers requalify first for the Linux guest. The order of `nix`,
  `flox`, `devbox`, `devenv`, `mise` and `pixi` determines what the parity test
  can cover first. mise and pixi download artifacts during preparation, so their
  registries enter the preparation egress, and pixi's macOS-only `SDKROOT`
  grants have no Linux counterpart.
- Whether the published crate ships the guest runtime build, or the artifact
  channel from `distribution.md` does.

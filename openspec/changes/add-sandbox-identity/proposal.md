# Change: add-sandbox-identity

## Why

Inside a sandbox, devcroft runs everything as the user who ran `up`: same
name, same uid, same gid (`sandbox_identity`, `src/lifecycle/up.rs:2446`;
`devcroft` appears only as a fallback name when the host lookup fails). The
fleet init helper maps `0 → caller` and later returns to the caller's identity
(`src/fleet/init.rs:462`, `src/fleet/mount.rs:132`). On the host, a sandbox's
processes therefore carry the user's own DAC authority, limited only by
Landlock or Seatbelt.

The owner asked (2026-11-20, `new/native-execution-identity-2026-11-20.md`)
for devcroft to run the inside of every sandbox under **its own user**, named
`devcroft` or configured, **never root**, and — as the default, natively too —
under a **host uid different from the user's**, with the shared-uid form kept
as an explicit alternative. Two further facts make it pressing now:

- The base-runtime changes put devcroft's pinned runtime, namespace directory
  and macOS redirect library under `<project>/.devcroft/<name>/`, which the
  default `filesystem.allow = ["."]` makes writable from inside. An identity
  that does not own the environment is the clean way to keep the environment
  out of the workload's reach, on top of the policy rule `add-own-userland`
  adds.
- `docs/decisions.md` records "`remoteUser`, UID/GID mapping → not
  applicable" on the premise that sessions run as the real user. The owner's
  direction removes that premise; the decision is revisited here, not defended.

## What Changes

- **A dedicated identity inside every sandbox**: name `devcroft` by default,
  configurable; a non-zero uid and gid; the sandbox `HOME`; consistent
  `getpwuid`, `id`, `$USER`. No workload process — session, hook, service,
  agent — ever runs as uid 0 inside.
- **Two modes, selected explicitly**:
  - `separate` — **the default**: the dedicated identity maps to a host uid
    different from the user's, one per sandbox. Linux: a user namespace mapped
    into the user's delegated subordinate range. macOS: a dedicated local
    account per sandbox, prepared once by a privileged devcroft helper.
  - `shared`: the dedicated identity maps to the user's own host uid (Linux,
    user namespace); on macOS only the host user is available, and that is
    stated.
- **No silent fallback.** When `separate` is selected and its preconditions are
  missing, `up` refuses at layer `backend`, naming what is missing and the
  `shared` alternative. On a platform where `separate` is not implemented yet,
  `up` states the mode it uses.
- **The identity does not own the environment.** The closure, artifact-store
  objects, devcroft's runtime, the namespace directory, the redirect library
  and the policy artifacts are not writable by it, in either mode; the
  workspace and the sandbox `HOME` are.
- **Workspace access in `separate` mode** through an explicit, removable grant
  (ACLs, with inheritance so the user keeps editing what the sandbox creates);
  never `chown -R`, never world-writable. Fleet clones are created owned by the
  sandbox identity.
- **The control plane stays the user's**: the CLI, host-side provisioning, the
  state directory and the listeners `up` creates. The keeper receives what it
  needs as inherited descriptors.
- `status` and `policy --render` show the internal identity, the host identity
  and the mode.
- **BREAKING** (on platforms where `separate` ships): a host without the
  preconditions refuses `up` by default until prepared or switched to
  `shared`; files the sandbox creates in `separate` mode are owned by the
  sandbox uid, with an ACL for the user.

## Capabilities

### New Capabilities
- `sandbox-identity`: the identity processes run under inside a sandbox, its
  host mapping per mode, preconditions and refusal, ownership of the workspace
  and the environment, the control-plane boundary, and inspection.

### Modified Capabilities
- `config`: adds the `[identity]` table (`user`, `mode`).

## Impact

- `src/lifecycle/up.rs` (`sandbox_identity`, keeper spawn), `src/fleet/init.rs`
  and `src/fleet/mount.rs` (id maps, `return_to_host_identity`), `src/keeper/`
  (spawning as the sandbox identity), `src/lifecycle/state.rs` (state written by
  the control plane or through inherited descriptors).
- Linux: `newuidmap`/`newgidmap` and `/etc/subuid`/`/etc/subgid` become a
  precondition of `separate` mode. A recorded tension with "the process tier
  requires no external backend binary" (`use-nono-library`): these are identity
  helpers of the host, not a sandbox backend, and the owner decides.
- macOS: a privileged helper (the `devcroft` binary itself, registered with
  launchd) that creates and owns per-sandbox accounts and launches the keeper
  as them. A recorded tension with "the keeper SHALL NOT be executed as a child
  of a separate sandboxing binary" (`add-linux-agent-fleet`, `use-nono-library`):
  the helper is the same binary, and the owner decides. A signed package is
  needed, since `cargo install` cannot register a daemon.
- Swift (native mode): a separate account has none of the user's keychain or
  signing identities; ordinary and signing builds are qualified separately.
- `docs/decisions.md` (`remoteUser`, UID/GID mapping), `docs/threat-model.md`,
  `docs/known-gaps.md`, `devcroft doctor`.

## Context

Source: `new/native-execution-identity-2026-11-20.md` (the owner's request and
its decisions), the adversarial review of 2026-11-22 (which raised the costs
below), and the base-runtime changes committed the same day.

Owner's request, verbatim: "aș vrea ca devcroft să aibă alt user sub care să
ruleze în interior chiar nativ, să zicem devcroft sau configurabil", and on
being offered the two meanings: "a doua variantă ca default dar și prima
posibilă dacă face sens" — a different host uid by default, the shared uid as
an explicit alternative.

Current state (read from the code, 2026-11-22):

| Where | Today |
|---|---|
| `up`, Linux | `sandbox_identity` (`up.rs:2446`) takes the caller's uid, gid and name (fallback `devcroft`) and writes them into the view's `/etc/passwd`; the PID-namespace path maps `0 → caller` during setup and returns to the caller's identity in a nested namespace (`mount.rs:132`) |
| fleet, Linux | `write_id_maps` (`init.rs:462`) maps `0 → caller` for the init helper; agents return to the caller's identity |
| macOS | processes run as the user; no identity mechanism at all |
| Tests | `tests/up_pid_namespace.rs` asserts the session uid and that a file created inside is owned by the user outside — the current contract, which this change replaces for `separate` |
| `.devcroft/<name>/` | inside the project root, which the default `filesystem.allow = ["."]` grants read-write; nothing protects the runtime, namespace directory or policy artifacts from the workload |

## Goals / Non-Goals

**Goals:**
- A dedicated, non-root identity inside every sandbox, consistent across the
  APIs tools use to ask who they are.
- A host identity different from the user's, by default, on both platforms.
- The workload cannot modify its environment or devcroft's runtime.
- No mode change without the user seeing it.

**Non-Goals:**
- A security boundary. A different uid adds DAC on top of Landlock/Seatbelt; the
  threat model's claim stays "accident protection".
- Separate identities for workload and services. Recommended by the source
  document; the granularity (per sandbox, per agent, per role) is left open and
  is a follow-up.
- Changing the identity of host-side provisioning. It stays the user's, as the
  two-phase model requires.

## Decisions

### D1. One identity per sandbox, named inside, mapped per mode
The inside identity is always the dedicated one (name from `[identity] user`,
default `devcroft`; uid and gid fixed inside, proposed 1000). What changes per
mode is the host side of the mapping. This keeps tools' view stable across
modes and platforms where possible, and makes the mode a single, visible
choice.

### D2. Linux: user namespaces, subordinate ranges for `separate`
`separate`: the sandbox's user namespace maps the inside identity to a host uid
and gid taken from the user's `/etc/subuid` and `/etc/subgid` ranges, written
with `newuidmap`/`newgidmap` (an unprivileged process cannot map ids it does not
own). Each sandbox gets its own host id from the range; the allocation is
recorded in `Meta` and is not reused while the sandbox's files or processes
remain. Setup that needs inside-root (mounts, `chown` of the sandbox `HOME` to
the mapped id) runs before the restriction, in devcroft's own helper, as today.
`shared`: the existing mapping, with the inside identity renamed and renumbered
(`<inside uid> → caller`), no helper needed.
*Tension, for the owner:* `newuidmap` is an external setuid binary. The standing
requirement "the process tier requires no external backend binary" was written
about sandbox backends (nono vs. `nono wrap`); these are the host's identity
helpers. Either the requirement is reworded to say so, or `separate` is not the
default on Linux. This change assumes the former.

### D3. macOS: per-sandbox accounts through a devcroft-owned helper
macOS has no uid mapping. `separate` needs a real local account per sandbox (a
hidden `_devcroft-<n>` user in a `_devcroft` group), created once by a
privileged helper — the `devcroft` binary itself, registered with launchd via
`SMAppService`, speaking a narrow, authenticated protocol: create or release an
account, prepare ACLs, start the keeper as that account with the listeners
passed as descriptors. The keeper still applies Seatbelt to itself.
*Tensions, for the owner:* (a) the keeper becomes a child of the helper; the
standing requirement forbids "a separate sandboxing binary", and the helper is
the same binary, which this change reads as compliant; (b) `cargo install`
cannot register a daemon, so `separate` on macOS needs a signed package;
(c) Swift (native) under a fresh account has none of the user's keychain,
signing identities or privacy grants — ordinary builds and signing are
qualified separately, and `shared` remains available.
Until the helper ships, macOS runs `shared` and says so (the "not implemented
on this platform" branch of the no-fallback requirement).

### D4. The environment is read-only to the workload, in both modes
Policy: on Linux, the view mounts `.devcroft/<name>/` read-only except `home/`
(Landlock cannot carve a read-only hole into a read-write grant; the mount view
can). On macOS, Seatbelt denies writes under `.devcroft/<name>/` except `home/`
(last-rule-wins, the same technique as `scope_exec_to_read_grants`). Ownership:
in `separate` mode those files belong to the user and the sandbox identity has
only read access. The closure and artifact objects are already read-only grants.
*Implemented 2026-11-23, with one correction:* `home/` was not the only thing a
sandbox writes there. process-compose binds its socket and writes its log, and
an agent writes its report, so those moved to `run/`, writable too (`svc.sock`,
so the path is a byte shorter than before and no project nears the OS limit
because of it). macOS carves the two out of the denial (`require-not`) instead
of allowing them after it, which would have granted them where the manifest
does not grant the project. A sandbox named `swift` also keeps `cache/`, the
swift provider's. `up`'s own writes there follow no link
(`runtime::owned_dir`). Not covered: sibling directories under `.devcroft/`
(another sandbox of the project, a derived flox environment), which remain
writable with the project.
`add-own-userland` gains the matching requirement for the runtime and
namespace directory, so it holds even before this change ships.

### D5. Workspace access: ACLs with inheritance, removed by `rm`
`separate` mode: `up` adds an access entry for the sandbox identity on the
workspace, with inheritance, plus an inherited entry for the user, so files the
sandbox creates stay editable by the user (POSIX default ACLs on Linux,
`file_inherit,directory_inherit` ACEs on macOS). `rm` removes both. No ownership
change of existing files, no world-writable bits. git refuses repositories
owned by another uid ("dubious ownership"); the sandbox's git configuration
(in its `HOME`) marks the workspace `safe.directory`. Fleet clones are created
by the agent's identity and need no ACL.
*Alternatives:* a shared group with `setgid` directories (fragile with umask and
tools that reset modes); a private workspace with sync (a much larger product
change); `chown -R` (rejected by the owner's document).

### D6. Control plane keeps the user's identity; the keeper gets descriptors
The listener-before-restriction design already passes the sockets as inherited
descriptors, so a keeper running as another uid can serve them without the
socket directory changing owner. The same applies to its log and to any state it
writes: opened by `up`, passed down. The state directory and the data directory
stay the user's, mode 0700, and denied to the sandbox as today.

### D7. Revisit `decisions.md`
"`remoteUser`, UID/GID mapping → not applicable" is rewritten: the premise
(sessions run as the real user) no longer holds by default; the ownership
problems it avoided are handled by D5 and are the price of the default.

## Risks / Trade-offs

- [Onboarding: a fresh Linux host without subordinate ranges, or a Mac without
  the helper, refuses `up` by default] → `doctor` names the preparation; the
  error names `[identity] mode = "shared"`; on macOS `shared` is the platform
  default until the helper ships.
- [Editors on the host editing workspace files owned by the sandbox uid] →
  inherited ACL for the user; measured with VS Code, JetBrains and git on both
  platforms.
- [Tools that check ownership (`git`, `ssh` keys in `HOME`, `npm` caches)] →
  the sandbox `HOME` is owned by the sandbox identity; `safe.directory` for git;
  measured per qualified workload.
- [Subordinate-range exhaustion or collisions with other rootless tools] →
  allocation recorded, checked against the range, refused with a message rather
  than overlapping.
- [macOS account sprawl] → accounts released by `rm`; a pool bounded by
  configuration; listed by `doctor`.

## Migration Plan

- Existing sandboxes keep their current identity until `up --recreate`; `status`
  shows "identity: legacy (user's uid)".
- On recreate in `separate` mode, existing files in the workspace keep their
  owner; `up` adds the ACLs. The sandbox `HOME` is re-owned to the sandbox
  identity by devcroft's helper.
- `tests/up_pid_namespace.rs` is split: the `shared` contract (file owned by the
  user outside) and the `separate` contract (file owned by the sandbox's host
  uid, editable by the user through the ACL).

## Open Questions

- Should a repository be able to choose `shared` in its manifest, or only the
  user (a CLI flag or a user-level configuration)? `shared` lends the user's own
  uid, not another account, so the manifest key is proposed; the owner decides.
- Workload versus services under distinct identities: granularity and timing.
- The inside uid: fixed 1000, or chosen to avoid collisions with uids baked into
  tool caches.

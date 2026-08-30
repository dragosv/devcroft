# Add Mount Isolation

**Status:** proposed. Closes a measured gap in every sandbox devcroft runs
today, not a future one.

## Why

**Landlock does not mediate `connect()` to a unix socket.** Its network
rules cover TCP for AF_INET/AF_INET6 only; AF_UNIX falls through to
ordinary filesystem permissions. A sandboxed process therefore reaches any
unix socket whose DAC allows it — *including sockets in directories the
compiled policy explicitly does not grant*.

Measured, not inferred (`tests/unix_socket_not_mediated.rs`): a real
Landlock-restricted process granted only its cwd connects to a socket
under `/tmp`, and to `/nix/var/nix/daemon-socket/socket` with `/nix`
ungranted. That socket is `srw-rw-rw-` under nix's multi-user model, so
the sandbox holds whatever authority the nix daemon extends to a local
user: realizing store paths, building derivations. That is exactly the
package-manager authority `sandbox-provisioning` P2a/P2b says an agent
must not have, and that change's design.md asserted a hook "does not
silently receive a writable `/nix` or the daemon socket" — the second half
of which was false and has since been corrected.

**The gap is a class, not one socket.** A nix daemon is the instance
present on this host and it is comparatively benign, because nix
deliberately makes that socket world-accessible and the daemon enforces
its own protocol. A Docker socket, a systemd private socket, or a
dbus session bus reachable the same way would be far worse, and devcroft's
policy would not stop any of them.

**No Landlock ABI expresses this**, so the fix cannot be another rule.
Two mechanisms can: seccomp filtering on `connect()`, or not having the
path in the sandbox's mount view at all. This change takes the second.
Measured: masking a path inside an unprivileged
`unshare(CLONE_NEWUSER | CLONE_NEWNS)` turns the connect into `No such
file or directory`. Nothing to filter, because there is nothing left to
name — and it closes the class rather than one socket at a time.

There is a second, independent reason to want this, which is why the
change is framed as a capability rather than a patch: **Landlock hides
nothing.** A sandbox can enumerate the entire filesystem tree, learning
paths, project names, and usernames it cannot read. A mount namespace
makes the view itself minimal, which is the difference between "you may
not read this" and "this does not exist here".

## What Changes

- **NEW** `filesystem-view`: each sandbox gets its own mount namespace,
  containing its project root, its provider's resolved runtime paths, a
  minimal system layer, a private `/tmp`, and nothing else.
- **MODIFIED** `policy`: the view is compiled and inspectable like every
  other rule, so `policy --render` shows what a sandbox can *see*
  alongside what it may access.
- **MODIFIED** `cli`: `doctor` reports whether mount isolation is
  available on this host, as it already does for network namespaces.

## Impact

- Affected specs: new `filesystem-view`; modified `policy`, `cli`.
- Affected code: `src/fleet/` (the namespace primitive, alongside
  `netns`), `src/lifecycle/up.rs` (entered in the same `pre_exec` that
  already enters the network namespace), `src/policy/` (compiling and
  rendering the view).
- **`add-linux-agent-fleet` task group 2 consumes this rather than
  duplicating it.** That change already lists "implement the mount plan:
  read-only system layer with merged-`/usr` symlinks, private `/proc`,
  minimal `/dev`, private `/tmp`, workspace bind" — the same work, scoped
  to agents. Splitting it out follows the precedent set by `fleet::netns`,
  which shipped for single sandboxes first and left fleet a second
  consumer of one primitive.
- **`sandbox-provisioning` P2a/P2b** stops being enforced solely by
  devcroft's refusal to grant, and becomes a boundary the kernel holds.

## Non-Goals

- **Not a container.** The view is constructed for isolation, not for
  image portability. There is no layering, no image format, no registry.
- **Not a replacement for the policy.** Landlock still governs access to
  what is visible; this governs what is visible at all. Removing either
  would weaken the result.
- **Not bubblewrap.** See design.md M2 — the capability is wanted, the
  binary is refused by two standing requirements.

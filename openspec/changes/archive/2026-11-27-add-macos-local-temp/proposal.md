# Change: add-macos-local-temp

## Why

On Linux a sandbox has a `/tmp` of its own, a private tmpfs, and its own `HOME`.
On macOS only the `HOME` is its own: `/tmp` is the host's, read-write, because
the baseline grants it and there is no mount to put anything else there.

Measured on macOS arm64, 2026-11-24, two flox sandboxes (`design.md`):

- one sandbox read a file the other wrote in `/tmp`;
- a sandbox read a file of the host's there, and **deleted** it;
- `TMPDIR` is unset, and the runtime's `mktemp` fails in every sandbox: without
  it, it uses `/var/folders/...`, which the policy refuses.

`HOME` is right for what reads the variable, and wrong for what asks the user
database: `getpwuid` answers the host's name and `/Users/<name>`, which is
refused.

`add-macos-exec-redirect` made one thing possible that was not when the `/tmp`
grant was added: devcroft has code inside the sandbox's processes, and can
answer for a path.

## What Changes

- **A temp directory per sandbox on macOS**, read-write, mode 0700, at a short
  path outside the project; `TMPDIR`, `TMP` and `TEMP` name it. Emptied by
  `down`, removed by `rm`.
- **The baseline no longer grants the host's `/tmp` on macOS.** **BREAKING** for
  a manifest that relies on sharing it; `filesystem.allow = ["/tmp"]` gives the
  host's back, under the manifest's own origin.
- **Literal `/tmp` and `/private/tmp` paths reach that directory**, through the
  library that redirects execs (owner's choice, 2026-11-24). Where a process
  lost it, or a call is not covered, they are refused, as base-name paths are;
  never the host's. The host's grant is removed only once this works.
- **`HOME` stays as it is** (local, read-write, kept across `down`, removed by
  `rm`), and `getpwuid` and `getpwnam` answer with it and with the sandbox's
  user name.
- **devenv uses a separate dotfile and runtime directory per macOS sandbox.**
  Hooks, services, and sessions get the same paths. Scripts that hardcode
  `.devenv/run` must use `$DEVENV_RUNTIME` or `$DEVENV_DOTFILE/run` on macOS.
- Linux is unchanged, except that `TMPDIR` is set to `/tmp` where the
  environment does not name a usable one.

## Capabilities

### New Capabilities
- `sandbox-temp`: the sandbox's own temp directory, its lifetime, and how
  programs reach it.

## Impact

- `src/policy/mod.rs` (`SANDBOX_TEMP_DIR`), `src/lifecycle/up.rs`,
  `src/lifecycle/terminate.rs`, `src/runtime/redirect.c`, `docs/known-gaps.md`.
- Providers that name a path under `/tmp` (devenv's `/tmp/devenv-<hash>`) and
  services whose sockets live there.
- Swift (native mode, no redirect) keeps its own `TMPDIR`.

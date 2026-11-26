# Change: add-macos-exec-redirect

## Why

On macOS, nothing inside a sandbox can use a shell or utility by absolute
path: `#!/bin/sh`, `#!/usr/bin/env`, `system()` and `popen()` all fail with
`EPERM`, because Seatbelt correctly refuses the host's `/bin` and `/usr/bin`
and there is no view to put devcroft's own entries in (measured 2026-11-22;
`initdb` reports `postgres` missing because its `popen` never runs). Linux gets
the absolute-path namespace from the mount view in `add-own-userland`. macOS has
no per-process path overlay.

The owner's requirement (2026-11-22): absolute paths must **work** on macOS as
on Linux, resolving to devcroft's runtime, whatever macOS needs for it, and
without touching the workspace. Measured the same day under a real devcroft
sandbox: a small interposition library loaded only into sandbox processes
redirects `/bin/sh`, `/bin/<name>` and `/usr/bin/env` to the runtime —
`system()`, `popen()`, shebang scripts and a Nix closure's bash all reached the
runtime — while every absolute path outside it stayed `EPERM`, never the host.

## What Changes

- An exec-redirect library, built into the `devcroft` binary and written next
  to the runtime at `up`, interposes `posix_spawn` and the `exec` family. An
  exec of a base-name path (`/bin/<name>`, `/usr/bin/<name>`, `/bin/sh`,
  `/usr/bin/env`) that has an entry in the sandbox's namespace directory is
  redirected to that entry — the provider's executable where the environment
  supplies the name, the runtime's otherwise, exactly as `add-own-userland`
  computes it for Linux (rule B: an absolute path resolves as its name does). A
  script whose `#!` interpreter is such a path is started with that entry.
  Everything else is left to Seatbelt, unchanged.
- The keeper puts the library into the environment of every process it starts
  in the sandbox — sessions, devcroft hooks, confined activation, and services —
  before the process starts.
- `/bin/sh` on macOS resolves to the sandbox's resolved shell, as on Linux: the
  environment's bash in nix, devbox and flox sandboxes, the runtime shell only
  where no shell is supplied (`embed-brush-runtime`).
- The library is part of the runtime generation and has a read grant with
  origin `runtime:gen-N`.
- Published limits: Apple-signed and hardened-runtime binaries drop the
  library, and processes that clear their environment lose it; in every such
  case the result is `EPERM`, never the host. Swift stays native.
- `docs/known-gaps.md`: the macOS absolute-path entry from
  `add-runtime-provenance` is replaced by these limits.

## Capabilities

### New Capabilities
- (none new)

### Modified Capabilities
- `base-runtime`: adds the macOS mechanism for the absolute-path namespace and
  its published limits.

## Impact

- New: a small C source compiled with the `cc` crate in `build.rs` on macOS
  targets, embedded in the binary, written at `up` under
  `<project>/.devcroft/<name>/runtime/`, ad-hoc signed by the linker.
- `src/lifecycle/up.rs`, `src/keeper/session.rs`, `src/lifecycle/hooks.rs`,
  `src/services/mod.rs`: environment injection at every spawn point, through
  the shared environment builder (`KeeperEnv`).
- `src/policy/`: the library's read grant.
- macOS only. Linux is unaffected.
- Build: `cc` as a build dependency on macOS; Command Line Tools are already
  required by `cargo install` there.

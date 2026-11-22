# Change: embed-brush-runtime

## Why

After `add-own-userland` and `add-macos-exec-redirect`, every base name in a
sandbox resolves the same way by name and by absolute path: the provider's
executable where the environment supplies one, devcroft's runtime otherwise
(rule "B", owner's choice 2026-11-22). The runtime has utilities but no shell,
so one host dependency is left:

- **Environments without a shell.** mise and pixi environments that install
  none get the host's `/bin/sh` as a file grant on Linux and the whole of
  `/bin` plus `/var/select` on macOS. It is the last `host:` entry the
  provenance witness reports for non-native providers.

Every closure-tier environment already has a shell, pinned by its lock
(measured 2026-11-22): nix and devbox through stdenv's bash on `PATH`, flox and
devenv through a bash in the closure's requisites, which `src/shell.rs` picks.
Under rule B that shell stays at `/bin/sh`; brush does not replace it.

One more measured fact makes brush worth having beyond that gap: stdenv's
`bash`, which nix and devbox sessions log into, is nixpkgs' non-interactive
build — no line editing in an SSH session. An interactive brush session is a
real improvement there.

brush is not free of defects. Measured on brush-shell 0.4.0 / brush-core 0.5.0
(`brush-functional-corrections-2026-11-20`, Linux x86_64; BR-01 and BR-02 also
on macOS arm64): signal traps never run, `trap ''` does not ignore, `$!` is
empty, `wait PID` is unimplemented, `kill PID` sends `KILL`; invoked as `sh`, it
does not enter POSIX mode. For the environments this change targets, the only
alternative is the host's shell, which the invariant forbids, so the defects are
published with their remedy (declare a shell in the environment) rather than
blocking the change.

## What Changes

- **brush embedded** through `brush_core`, run in a dedicated child process
  mode, never in the keeper; no ambient startup sources (profiles, rc files,
  `ENV`, `BASH_ENV`, brush's own config file); POSIX `sh` semantics when
  invoked as `sh`.
- **Slice A — runtime shell where the environment has none.** Shell resolution
  becomes: the environment's `PATH`, then the closure's requisites, then the
  runtime shell. The common runtime layer never grants a host shell.
  **BREAKING (mise, pixi):** the host shell grant is removed on Linux, and
  `/bin` + `/var/select` on macOS; `#!/bin/bash` scripts in those projects stop
  working unless the environment provides bash (pixi: `bash` from conda-forge).
- **Slice B — interactive fallback.** `devcroft shell` and SSH login use the
  runtime shell when `$SHELL` selects none under today's `exec` contract.
  History is kept under the sandbox `HOME` and saved on disconnect and on
  `down`, not only on clean exit.
- **The runtime shell never displaces a shell the environment supplies** at
  `/bin/sh` or by name.
- **Known defects are published until qualification is recorded** against the
  pinned brush revision (traps, `trap ''`, `$!`, `wait PID`, `kill` default,
  POSIX mode, startup time).
- **Explicit interpreters are never redirected.** `/bin/bash` is not a base
  name; `#!/usr/bin/env bash` and `bash script` reach the environment's bash.
- The brush revision is part of the runtime generation.

## Capabilities

### New Capabilities
- (none new)

### Modified Capabilities
- `base-runtime`: adds the runtime shell, its process model and startup
  policy, its place in shell resolution, the interactive fallback, defect
  publication, and the explicit-interpreter rule.

## Impact

- `Cargo.toml`: `brush-core` (and `brush-builtins`) promoted to
  `[dependencies]`; a second `uucore` version possible until brush updates.
- `src/bin/devcroft.rs` / `src/runtime/`: `sh` dispatch and the dedicated shell
  child mode; interactive path with basic line input on terminals that do not
  answer cursor queries (the lesson in `examples/test-row-sh.rs`).
- `src/shell.rs`: a third resolution step (runtime shell) after requisites;
  `Meta.shell` keeps the launch form (`sh` link), not the canonical binary path,
  for the runtime shell.
- `src/provider/mise.rs`, `src/provider/pixi.rs`: host shell grants removed.
- `src/exec.rs`, `src/ssh/server.rs`: interactive fallback.
- Upstream: issues BR-01–BR-10 (drafted in
  `new/brush-functional-corrections-2026-11-20.md`), filed by the owner.

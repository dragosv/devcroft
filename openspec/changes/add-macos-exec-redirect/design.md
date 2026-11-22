## Context

Third of four changes carrying out
`new/devcroft-runtime-absolute-paths-design-2026-11-22.md`. macOS has no
per-process path overlay, so the Linux mechanism (`add-own-userland`, a tmpfs
`/bin` and `/usr/bin` in the mount view) has no direct counterpart.

Measured 2026-11-22 on macOS 15.8.3 arm64:

**Mechanisms that fail for this goal**

| Mechanism | Finding |
|---|---|
| Seatbelt | allows or refuses; no path redirection |
| `/var/select/sh` | `/bin/sh` is a trampoline (`PROJECT:dash-16`) that accepts only `/bin/bash`, `/bin/dash`, `/bin/zsh` ("Unrecognized shell referenced in %s: %s"); also global to the host |
| Endpoint Security | authorises or denies exec; cannot redirect |
| chroot + passthrough | replaces all of `/`: the workspace, `/nix` (its own APFS volume, disk3s7), Xcode and the dyld shared cache (`libSystem` is not on disk; arm64e cache ≈ 4.7 GB on the Preboot volume) must be re-provided; needs root, a privileged helper, and a passthrough file system (FSKit on macOS 26, or loopback NFS) with known watcher problems. Out of proportion for "only `/bin/sh` and userland" |

**dyld interposition, measured under a real devcroft sandbox** (devbox
provider, Seatbelt backend, `devcroft exec`; runtime = the repo's brush fixture
with `sh`, `echo`, `pwd`, `sleep`):

| Case | Without | With |
|---|---|---|
| `system()` / `popen()` → `/bin/sh` | refused | runtime shell |
| `#!/bin/sh` via `posix_spawn` from a C program | `EPERM` | runtime shell (shebang emulated) |
| `#!/bin/sh` from the session shell | `EPERM` | runtime shell |
| a Nix closure's bash executing `/bin/sh` | — | runtime shell |
| `/bin/echo`, `/bin/pwd` (in the runtime) | — | runtime |
| `/bin/ls` (not in the runtime) | — | `EPERM`, not the host |
| `/usr/bin/make` (Apple) | — | `EPERM`, not the host |

Outside a sandbox, `/usr/bin/make` (SIP-protected) dropped the library and ran
the host's bash; inside a sandbox the same binary cannot start. The first
in-sandbox run failed for the commands run directly by the script's shell,
because that shell had started before the variable was exported — the reason
the keeper must set it before spawning.

## Goals / Non-Goals

**Goals:**
- Absolute-path shell and userland work on macOS and resolve as their names do:
  the environment's executable where it supplies one, devcroft's runtime
  otherwise (rule B).
- The workspace and everything else are untouched; no root, no helper, no
  macOS-version floor beyond what devcroft already supports.
- Failure where the redirect cannot apply is `EPERM`, never the host.

**Non-Goals:**
- Covering Apple-signed or hardened-runtime binaries. Swift stays native.
- Redirecting `stat`, `open` or `access` on those paths. Exec only, until a
  workload shows the need.
- A security boundary. Seatbelt remains the boundary.

## Decisions

### D1. dyld interposition of `posix_spawn` and the exec family
A Mach-O dylib with an `__interpose` section, loaded through
`DYLD_INSERT_LIBRARIES`, replaces `posix_spawn`, `posix_spawnp` (absolute-path
case), `execve` and the `execv*` family. Measured: calls made from inside
`libSystem` (`system()`, `popen()`) are interposed. Mapping: a base-name path
(`/bin/<name>`, `/usr/bin/<name>`, `/bin/sh`, `/usr/bin/env`) with an entry in
the sandbox's namespace directory (`add-own-userland` D4) is replaced by that
entry; otherwise the call proceeds unchanged and Seatbelt decides. The library
holds no policy of its own: under rule B the entry is the provider's
executable where the environment supplies the name and the runtime's
otherwise, computed once at `up` for both platforms. The 2026-11-22 prototype
mapped into a fixed runtime directory; reading the namespace directory instead
is the same lookup with a different root.
*Alternatives:* see the Context table.

### D2. Shebang emulation
The kernel resolves `#!` itself, so an interposed exec of a script would still
reach the host interpreter (measured). The library reads the first line; when
the interpreter is in the namespace and the runtime provides it, it starts the
runtime entry with `[interpreter, optional argument, script, argv[1..]]`, the
kernel's convention. Anything it cannot parse is passed through unchanged.

### D3. Injected by the keeper at every spawn point
The variable is set in the one environment construction used for sessions,
hooks, confined activation and services (`KeeperEnv`), before spawn. devcroft's
own `env` and shell re-add it if a caller cleared it but kept running devcroft
code. `/bin/sh` maps to the namespace entry for `sh`, the shell resolved for the
sandbox, exactly as on Linux.

### D4. Built from C in `build.rs`, written at `up`
`cargo install` installs binaries only, so the dylib cannot be a separate
artifact. A small C source is compiled with the `cc` crate on macOS targets,
embedded as bytes, and written at `up` beside the runtime links, with a read
grant (`runtime:gen-N`). The linker ad-hoc signs it, which arm64 requires.
*Alternative:* a Rust `cdylib` in the same package — rejected, `cargo install`
does not install library targets, and a second package adds a publish step.

### D5. Limits are published, not compensated
Apple-signed and hardened-runtime binaries strip `DYLD_*`; processes that clear
the environment lose it. devcroft does not grant host shells to cover these;
the result is `EPERM`, consistent with the invariant. Swift's native mode is
the only route to host tools.

## Risks / Trade-offs

- [A notarised tool from mise or pixi uses the hardened runtime and loses the
  redirect] → measured per tool in group 0; listed in `known-gaps.md`.
- [Go binaries call `libSystem` through trampolines; interposition is expected
  but unmeasured] → group 0.
- [process-compose (Go) starts service commands] → it starts the absolute
  resolved shell path, not `/bin/sh`; measured in group 0 anyway.
- [Interposing exec changes `errno` or argument edge cases] → the library
  passes through anything it does not map and is tested against the exec
  family's error paths.
- [Seatbelt's `file-map-executable` refuses the library under some manifests]
  → it has its own grant; a manifest that grants nothing but its own paths is
  tested.

## Migration Plan

Additive on macOS. Sandboxes get the redirect on `up --recreate`. mise and pixi
keep their `/bin` grants until `embed-brush-runtime` gives them a runtime
shell; their utilities and `env` already come from the runtime after
`add-own-userland`.

## Open Questions

- Whether to extend the redirect to `stat`/`access` for `/bin/<name>` so
  `test -x /bin/ls` agrees with exec. Decide on evidence from real workloads.

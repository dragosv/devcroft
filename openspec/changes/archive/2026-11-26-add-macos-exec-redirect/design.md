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

### Group 0, measured 2026-11-23 (macOS 15.8.3 24H241, arm64)

`spikes/macos-exec-redirect/` reproduces everything below from the
repository (`run.sh`); the 2026-11-22 table above was reproduced line for line
first (task 0.1).

| Case | Result |
|---|---|
| Go 1.26.1 `exec.Command("/bin/sh", "-c", …)` | redirected |
| Go `exec.Command` of a `#!/bin/sh` script | redirected (shebang emulated) |
| Go `exec.Command("/bin/echo")` (in the runtime) | redirected |
| Go `exec.Command("/bin/ls")` (not in it) | `EPERM` |
| Go `syscall.Exec("/bin/sh", …)` | redirected |
| Node (`execSync`) | redirected |
| Python `subprocess.run(…, shell=True)` | reached the runtime shell; the prototype's log line did not appear in-sandbox (it does outside), unexplained |
| `make` from the closure | never touches `/bin/sh`: nixpkgs' make runs recipes with the store's bash |
| `npm run` | never touches `/bin/sh`: the script shell is found by name, the closure's `sh` |
| devbox postgres `initdb` from a `post_create` hook | ~26 `popen("/bin/sh")` calls redirected; `initdb` gets past the stage that used to report `postgres` missing and stops at `shmget` (the separate, documented Seatbelt gap) |
| process-compose service (flox, `python3 -m http.server`) | starts; the variables are in the service's environment (read host-side with `ps -E`) |
| a session started by a keeper whose environment has the variables | redirected with no `export` in the session (D3's shape) |

Tools' signing (task 0.3): conda-forge's `go`, `golangci-lint`, `curl`,
`make`, `jq` and `python3.12` (the pixi samples' tools) are ad-hoc signed
without the hardened runtime, and conda python kept the library and passed it
to its child. Node's official build (26.4.0) **has** the hardened runtime but
also the `allow-dyld-environment-variables` and `disable-library-validation`
entitlements, and kept the library. **Not measured:** the official
`go1.24.0.darwin-arm64` build the mise profile installs (its download and
injection were not run in this session).

In these runs the variables were set through `[env.vars]`, so the keeper
loaded the library itself as well (confirmed with `lsof`, 2026-11-23). That is
not a shape to ship; see D3.

The signing rows' "kept the library" was measured **outside** a sandbox.

**The remaining three, measured 2026-11-23** in sandboxes, same host, with the
library of `0cbfaa6` set through `[env.vars]` and a namespace whose `sh` is the
brush fixture (the runs are not in `run.sh`):

| Case | Result |
|---|---|
| flox service whose program calls `os.popen` | redirected |
| `cargo build --offline` with a `build.rs` running `/bin/sh` (nix) | redirected; `/bin/ls`, without an entry, `EPERM`. Needed `TMPDIR` inside the project (nix's known stale `TMPDIR`) |
| mise's official `go1.24.0.darwin-arm64` | **drops the library**: hardened runtime (`flags=0x10000`), no entitlements shown |
| a program built by that Go, started directly by the keeper | redirected |
| the same kind of program through `go run`, or through the host's `/bin/sh` | the variable does not reach it |

In a mise sandbox the host's `/bin` is granted until `embed-brush-runtime`, so
where the redirect is lost there the **host's** file runs, not `EPERM`
(`/bin/ls` listed `/`). The base-runtime record keeps saying `host:` for a
sandbox whose shell is the host's for that reason.

An x86_64 program under Rosetta was aborted by dyld (exit 134, outside a
sandbox) because the library was arm64 only; it is universal since. A missing
library aborts a process the same way, so removing `.devcroft/<name>/redirect/`
from inside breaks that sandbox's sessions until the next `up`.

**Stop criterion (0.6): reported, and the owner decided 2026-11-23 that it does
not block this change.** mise's one qualified tool
drops the library. Its ordinary workload (`go build`) runs today, on the host
shell the `/bin` grant allows. What its children do by absolute path
(`go run`, and by the same inheritance `go test` and `go generate`) is expected
to become `EPERM` once that grant goes; not measured, so `embed-brush-runtime`
measures `go test` and `go generate` before it removes the grant.

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
*Corrected 2026-11-23.* The variable is set by the keeper on each child it
spawns (`keeper::session::apply_activation_env`, the one path sessions, hooks,
confined activation and services share), and **never on the keeper or the
provisioning process**. This said `KeeperEnv`, which is the keeper's own
environment: dyld runs an inserted library before `main`, so before the keeper
restricts itself, and the library sits where the sandbox can write. The keeper
gets the two paths under names of its own (`DEVCROFT_REDIRECT_LIB`, `_NS`),
which are stripped from sessions. devcroft's
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

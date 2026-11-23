## 0. Measurement gate — answer before writing production code

Record each result in `design.md`, tagged measured, with macOS version and arch.

- [x] 0.1 Commit the 2026-11-22 prototype (`interpose.c`, the two test programs, the in-sandbox scripts) under `spikes/macos-exec-redirect/`, so the evidence in `design.md` is reproducible from the repo.
- [x] 0.2 Go: a Go binary in a sandbox calling `exec.Command("/bin/sh", "-c", …)` and `os/exec` of a `#!/bin/sh` script — redirected or not.
  - Redirected, `syscall.Exec` too; `/bin/ls` stays `EPERM` (`goprobe/`, design.md "Group 0").
- [ ] 0.3 Artifact tools: the notarised tools used by the mise and pixi samples (and Node's official build) — does each keep `DYLD_INSERT_LIBRARIES`; does `/bin/sh` from each reach the runtime or `EPERM`.
  - pixi's conda-forge tools: ad-hoc, no hardened runtime, keep it. Node 26.4.0: hardened but entitled, keeps it. **Open:** mise's official `go1.24.0.darwin-arm64`.
- [ ] 0.4 process-compose (Go) starting a service whose command is a shell line; services that themselves `popen`.
  - Shell-line service starts with the variables in its environment (flox). **Open:** a service whose own program `popen`s.
- [ ] 0.5 Real workloads through `/bin/sh`: `npm run` with a script, `make` from a closure, `cargo build` with a `build.rs` that runs `sh`, `python -c 'import subprocess; subprocess.run("true", shell=True)'`.
  - Python and Node redirected; `make` and `npm run` never reach `/bin/sh` (closure shell by name or store path); `initdb`'s `popen` redirected. **Open:** `cargo build` with a `build.rs`.
- [ ] 0.6 Stop criterion: if a qualified provider's ordinary workload cannot run because its tools drop the library, report to the owner before group 1.

## 1. The library

- [x] 1.1 C source: interpose `posix_spawn`, `posix_spawnp` (absolute path), `execve`, `execv`, `execvp` (absolute path), `execl*` as needed; map only base-name paths that have an entry in the sandbox's namespace directory (location passed by the keeper); shebang emulation with the kernel's argument convention; no logging in production.
  - `src/runtime/redirect.c`: `posix_spawn`, `posix_spawnp` and `execvp` (with a `/`), `execve`, `execv`; the `execl*` family reaches `execv`/`execve` inside libSystem, where calls are interposed (measured for `system()`/`popen()`, not separately for `execl`). Namespace from `DEVCROFT_NS`; stack only, no malloc (vfork-safe).
- [x] 1.2 `build.rs` compiles it with `cc` on macOS targets and embeds the bytes; other targets embed nothing.
  - `cc` 1.4 as a build-dependency (already in the lock); `-Werror`; linker ad-hoc signed; `build.rs` and the C file added to the packaging allowlist (`cargo package --list` shows both).
- [x] 1.3 Unit tests (macOS, guarded): mapping table, shebang parsing (interpreter with and without argument, CRLF, no trailing newline, unparseable header passes through), error passthrough (`ENOENT`, `EACCES`, `E2BIG`).
  - `tests/redirect_library.rs` (macOS, outside a sandbox, so passthrough is observable as the host's file running): 6 tests, all pass.

## 2. Integration

- [x] 2.1 Write the library at `up` beside the runtime links; read grant with origin `runtime:gen-N`; part of the generation identity.
  - `.devcroft/<name>/redirect/`, a directory of its own so the runtime directory stays exactly the inventory.
- [x] 2.2 Set the variable in the shared environment construction for sessions, `exec`, SSH, hooks, confined activation and services, before spawn.
  - On children only, not in `KeeperEnv` (design.md D3, corrected). Sessions and a `post_create` hook tested in a sandbox; services, SSH and activation share the spawn path and are not tested separately.
- [x] 2.3 The library resolves through the namespace directory built by `add-own-userland` (rule B): `/bin/sh` to the resolved shell, every other base name to the provider's executable or the runtime's, whichever the name resolves to.
  - Tested on the nix row: `/bin/ls` is the closure's GNU `ls`.
- [x] 2.4 The runtime's `env` and shell restore the variable when invoked with a cleared environment.
  - `env` only: the runtime has no shell. The library recognises devcroft's own binary by inode and restores the variables it saved at load.
- [x] 2.5 Native mode (swift) gets no redirect.
  - Not materialised, granted or set for a native provider. Covered by code, not by a test of its own.

## 3. Tests

- [ ] 3.1 macOS provider-row test, in a real sandbox: `system()`, `popen()`, `#!/bin/sh` from C and from the session shell, `#!/usr/bin/env python3`, a closure binary executing `/bin/sh`; the consistency check from `add-own-userland` 4.5 (`/bin/<name>` and `<name>` run the same file: GNU from the closure on the nix row, the runtime on a flox-minimal row); and a base name with no entry, `/usr/bin/make` and `/usr/bin/python3` fail with `EPERM`.
- [ ] 3.2 A hook test: a `post_create` hook whose program uses `popen`.
- [ ] 3.3 Update the provenance witness: on macOS, absolute base-name paths stop being `absent` and match their by-name provenance (`closure:` or `runtime:gen-N`) for non-native sandboxes.

## 4. Documentation

- [ ] 4.1 `docs/known-gaps.md`: replace the macOS `EPERM` entry with the redirect's limits (Apple-signed, hardened runtime, cleared environment), each with its measured examples.
- [ ] 4.2 `docs/threat-model.md`: one sentence — the redirect is compatibility, not a boundary.

## 5. Verification

- [ ] 5.1 `cargo build` on macOS and Linux (Linux embeds nothing), `fmt`, `clippy`, `doc --no-deps`, `test`, skip inventory recorded.
- [ ] 5.2 `openspec validate add-macos-exec-redirect --type change`.

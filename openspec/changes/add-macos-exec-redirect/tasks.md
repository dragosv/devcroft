## 0. Measurement gate — answer before writing production code

Record each result in `design.md`, tagged measured, with macOS version and arch.

- [ ] 0.1 Commit the 2026-11-22 prototype (`interpose.c`, the two test programs, the in-sandbox scripts) under `spikes/macos-exec-redirect/`, so the evidence in `design.md` is reproducible from the repo.
- [ ] 0.2 Go: a Go binary in a sandbox calling `exec.Command("/bin/sh", "-c", …)` and `os/exec` of a `#!/bin/sh` script — redirected or not.
- [ ] 0.3 Artifact tools: the notarised tools used by the mise and pixi samples (and Node's official build) — does each keep `DYLD_INSERT_LIBRARIES`; does `/bin/sh` from each reach the runtime or `EPERM`.
- [ ] 0.4 process-compose (Go) starting a service whose command is a shell line; services that themselves `popen`.
- [ ] 0.5 Real workloads through `/bin/sh`: `npm run` with a script, `make` from a closure, `cargo build` with a `build.rs` that runs `sh`, `python -c 'import subprocess; subprocess.run("true", shell=True)'`.
- [ ] 0.6 Stop criterion: if a qualified provider's ordinary workload cannot run because its tools drop the library, report to the owner before group 1.

## 1. The library

- [ ] 1.1 C source: interpose `posix_spawn`, `posix_spawnp` (absolute path), `execve`, `execv`, `execvp` (absolute path), `execl*` as needed; map only base-name paths that have an entry in the sandbox's namespace directory (location passed by the keeper); shebang emulation with the kernel's argument convention; no logging in production.
- [ ] 1.2 `build.rs` compiles it with `cc` on macOS targets and embeds the bytes; other targets embed nothing.
- [ ] 1.3 Unit tests (macOS, guarded): mapping table, shebang parsing (interpreter with and without argument, CRLF, no trailing newline, unparseable header passes through), error passthrough (`ENOENT`, `EACCES`, `E2BIG`).

## 2. Integration

- [ ] 2.1 Write the library at `up` beside the runtime links; read grant with origin `runtime:gen-N`; part of the generation identity.
- [ ] 2.2 Set the variable in the shared environment construction for sessions, `exec`, SSH, hooks, confined activation and services, before spawn.
- [ ] 2.3 The library resolves through the namespace directory built by `add-own-userland` (rule B): `/bin/sh` to the resolved shell, every other base name to the provider's executable or the runtime's, whichever the name resolves to.
- [ ] 2.4 The runtime's `env` and shell restore the variable when invoked with a cleared environment.
- [ ] 2.5 Native mode (swift) gets no redirect.

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

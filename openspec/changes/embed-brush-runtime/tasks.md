## 0. Upstream and evidence

- [ ] 0.1 Owner: file the brush issues drafted in `new/brush-functional-corrections-2026-11-20.md` (BR-01–BR-10). External action; not done by an agent.
- [ ] 0.2 Commit the spike package (`spikes/brush-uutils-native/`, `examples/brush-native-probe.rs`) from the owner's Linux host, selectively, so the qualification probes in group 4 re-run from the repo.
- [ ] 0.3 Turn the supplemental observations (`$!`, non-child `wait`, `kill` default, `EXIT` trap) into automated probes in that package.
- [ ] 0.4 Measure devenv's resolved shell on Linux and macOS (not measured 2026-11-22) and confirm it comes from requisites.

## 1. The runtime shell

- [ ] 1.1 Promote `brush-core` and `brush-builtins` to `[dependencies]` at a pinned revision; record source revision and lockfile hash in the generation identity.
- [ ] 1.2 Adaptor: `Shell::builder` with profile and rc skipped, no `ENV`/`BASH_ENV`/config-file evaluation, explicit cwd, name and positional arguments, `run_dash_c_command`, current-thread Tokio, explicit mapping of exit, signal termination and interpreter errors.
- [ ] 1.3 `sh` dispatch through the multicall, in a dedicated child; POSIX mode on when invoked as `sh`.
- [ ] 1.4 Tests: hostile `.bashrc`, `.profile`, `BASH_ENV`, `ENV` and brush config file write no marker; `set -o` shows posix on as `sh`; quoting, positional arguments, exit codes and `SIGPIPE` controls from the spike stay green.

## 2. Slice A — runtime shell where the environment has none

- [ ] 2.1 `src/shell.rs`: third resolution step (runtime shell) after requisites; `Meta.shell` keeps the launch form (`sh` link) for it.
- [ ] 2.2 Remove the host shell file grant (Linux) and `/bin` + `/var/select` (macOS) from the mise and pixi paths; the common layer never grants a host shell.
- [ ] 2.2a Before 2.2 removes the host shell grant on macOS: measure `go test` and `go generate` in a mise sandbox without it. mise's official Go is hardened and drops the exec redirect, so its children may meet `EPERM` at `/bin/sh` (`add-macos-exec-redirect` 0.6). If ordinary workloads break, report to the owner first.
- [ ] 2.3 The namespace entry for `sh` follows the resolved shell, so `/bin/sh` and `sh` agree on every row.
- [ ] 2.4 Provenance witness: mise and pixi `sh` and `/bin/sh` become `runtime:gen-N`; nix, devbox, flox keep `closure:`; no `host:` entry remains for any non-native provider.
- [ ] 2.5 Tests: a pixi sample without bash runs its activation and a `#!/bin/sh` script; adding `bash` from conda-forge makes `/bin/sh` that bash; `#!/bin/bash` fails without bash and no host bash runs; `tests/mise_provider_e2e.rs` still passes; a nix sandbox's `/bin/sh` is still the closure's bash.
- [ ] 2.6 `up` prints the removed host grants once for affected projects; README and `docs/known-gaps.md` note the BREAKING change and the remedy.

## 3. Slice B — interactive fallback (after the HUP/TERM part of BR-01 is fixed upstream)

- [ ] 3.1 `devcroft shell` and SSH login use the runtime shell when `$SHELL` selects none, under the existing `exec` contract.
- [ ] 3.2 Basic line input when the terminal does not answer cursor-position queries.
- [ ] 3.3 History under the sandbox `HOME`, saved on disconnect (within the keeper's grace) and on `down`; tests for both, plus persistence across `down`/`up`.

## 4. Qualification records — per pinned revision

- [ ] 4.1 Trap handlers on a real `TERM`: in a builtin-only loop, during `read`, during `wait`.
- [ ] 4.2 `trap ''` ignores `TERM`, `HUP`, `INT`, each in a fresh process.
- [ ] 4.3 `$!` non-empty and valid; `wait "$!"` returns 0 and 7 for children exiting 0 and 7; non-child `wait` returns 127.
- [ ] 4.4 Bare `kill PID` sends `TERM` to an independent target.
- [ ] 4.5 POSIX mode as `sh`; differences from dash on a small POSIX corpus recorded.
- [ ] 4.6 Startup budget declared (proposed: `brush_core` `-c :` median ≤ 2× bash on the same host) and measured on Linux and macOS.
- [ ] 4.7 Record each result against the revision and executable hash in `docs/base-runtime.md`; every unqualified behaviour listed in `docs/known-gaps.md` for runtime-shell sandboxes, and surfaced by `status` there.

## 5. Documentation and verification

- [ ] 5.1 `docs/base-runtime.md`: the shell, its place in resolution, its startup policy, the slices and the qualification records.
- [ ] 5.2 Regenerate `THIRD-PARTY-LICENSES.md`; measure the integrated binary size.
- [ ] 5.3 `cargo build`, `fmt`, `clippy`, `doc --no-deps`, `test`, skip inventory recorded.
- [ ] 5.4 `openspec validate embed-brush-runtime --type change`.

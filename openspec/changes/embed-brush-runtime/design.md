## Context

Fourth of four changes carrying out
`new/devcroft-runtime-absolute-paths-design-2026-11-22.md`, as amended by the
owner's choice of **rule B** (2026-11-22): an absolute path resolves as its name
does — the provider's environment first, devcroft's runtime where it supplies
nothing. After `add-own-userland` (runtime utilities, generation pin, namespace
directory, Linux view, common layer, Swift native) and `add-macos-exec-redirect`
(macOS redirect into the same namespace), the shell is the remaining piece.

What environments supply, measured 2026-11-22 on macOS inside real sandboxes:

| Provider | Shell resolved today | Source |
|---|---|---|
| nix | `bash-5.3p15` | environment `PATH` (stdenv) |
| devbox | `bash-5.3p15` / `bash-5.3p20` | environment `PATH` (stdenv), with and without a toolchain |
| flox | `bash-5.3p9` / `bash-interactive-5.3p9` | closure requisites (the environment `PATH` has none) |
| devenv | `bash-interactive-5.3p15` | environment `PATH`, first entry (measured 2026-11-24, Linux aarch64 and macOS arm64) |
| mise, pixi | the tools' own `sh` if any, else **the host's** | provider fallback (`mise.rs:987–1012`, `pixi.rs:790–815`) |

So under rule B the runtime shell is needed at `/bin/sh` only for mise and pixi
environments without a shell (and any future environment with neither a `PATH`
shell nor a requisite one). stdenv's `bash` is nixpkgs' non-interactive build,
which is what nix and devbox SSH sessions log into today.

Brush evidence, restated:

- **Spike** (`brush-uutils-workload-runtime-spike-handoff-2026-11-20`, Linux
  x86_64, brush-shell 0.4.0 / brush-core 0.5.0 from the registry and a pinned
  Git revision): 196/198 of 213 probes pass; 13 keeper probes pass, including
  `#!/bin/sh`, PTY and SSH.
- **Defects** (`brush-functional-corrections-2026-11-20`): BR-01 trap handlers
  never run on real signals (also on macOS arm64: a `TERM` handler is skipped and
  the shell dies with 143 while bash runs it); BR-02 `trap ''` does not ignore;
  BR-03 `$!` empty; BR-04 `wait PID` unimplemented (status 99); BR-06 expanded
  `export` broken in the registry build, fixed in Git; BR-07 inconsistent
  `ENV`/`BASH_ENV` handling on the CLI path; BR-10 `kill PID` sends `KILL`.
  Source: `Traps::register_handler` only stores the command; handlers run for
  `EXIT`, `ERR` and `DEBUG` only.
- **Startup** (Linux, medians of 20): bash `-c :` 0.87 ms, brush CLI 3.99 ms,
  `brush_core` child 1.43 ms.
- **POSIX mode:** through an `sh` link, `set -o` shows `posix off`.

## Goals / Non-Goals

**Goals:**
- No host shell in any non-native sandbox.
- A better interactive shell where the environment's is the non-interactive
  build, with history that survives the ways devcroft ends sessions.

**Non-Goals:**
- Replacing a shell the environment or its closure supplies. Rule B.
- Serving explicit `bash` requests with brush.
- Brush for managed services; services keep their environment's shell.
- Fixing brush inside devcroft. Fixes go upstream; devcroft pins a revision.

## Decisions

### D1. `brush_core` in a dedicated child, not the CLI entry
`brush_shell::entry::run()` parses process arguments, ends the process and has
its own startup and configuration loading (BR-07). The adaptor builds a
`brush_core::Shell` with profile and rc loading skipped, explicit working
directory, name and positional arguments, `BashMode` builtins, and
`run_dash_c_command` on a Tokio runtime (current-thread was the plan; measured
2026-11-24, it deadlocks on a substitution larger than a pipe), in a child started from
the `devcroft` binary. It is also the faster path (1.43 ms vs 3.99 ms). Process
state (`umask`, signal dispositions, `exec`, terminal and process groups) is why
it never runs in the keeper.

### D2. Third step of shell resolution, after requisites
`src/shell.rs` keeps its two steps (environment `PATH`, then closure requisites)
and gains a third: the runtime shell. Requisites stay ahead of the runtime
because they are part of the closure the project's lock pins — "what the
environment supplies", transitively — and they are already measured working for
flox. The runtime shell therefore lands only where neither exists.
*Alternative:* runtime shell ahead of requisites for flox and devenv, whose
environments do not put a shell on `PATH`. Rejected: it would replace a
lock-pinned bash with a less complete shell for no reproducibility gain.

### D3. Two slices
- **A, environments without a shell** (mise, pixi without bash): the alternative
  is the host, which the invariant forbids. These providers have no services,
  which removes the surface where BR-01/02/10 bite hardest. Ships first, with the
  defects published and the remedy (declare `bash`) stated.
- **B, interactive fallback**: brush's line editing and history are the gain,
  most visibly over stdenv's non-interactive bash. History saving on disconnect
  and `down` depends on brush handling `HUP` and `TERM` itself, which is part of
  the same signal work upstream; slice B waits for that part.

The earlier slice "C" (`/bin/sh` = brush in every non-native sandbox) is
withdrawn by rule B.

### D4. Qualification is recorded per revision, not inferred from a version
Registry and Git builds both report 0.4.0/0.5.0 (handoff §5). Each behaviour in
the "known defects" requirement is recorded against a pinned source revision
and executable hash, with the probe that passed. Unqualified behaviours are
published wherever the runtime shell is `/bin/sh`.

### D5. `Meta.shell` keeps the launch form for the runtime shell
The runtime shell is reached through a link named `sh` so dispatch sees `sh`.
The resolver canonicalises today; storing the canonical `devcroft` path would
lose the name and send `-c` into the normal command parser (handoff §14.3). The
executable's identity (for policy) and its launch form (for dispatch) are kept
separately. Closure shells keep today's behaviour.

### D6. `/bin/bash` stays outside the namespace
A `#!/bin/bash` script asks for bash. Serving it with brush would break the
explicit-interpreter rule and hide every incompatibility as brush behaviour.
`#!/usr/bin/env bash` works through the namespace's `env` and `PATH`.

## Measured: 2.2a, Go in a macOS mise sandbox without the host shell (2026-11-24)

macOS 15 arm64, mise's official Go 1.24.0, at `a521a96` with two temporary
edits: `layer::resolve_shell` links `examples/test-row-sh` (brush) as `sh` and
grants no `/bin`, and mise does not grant `/usr/bin/env`. `spikes/brush-runtime/`.

| Workload | Today | Without the host shell |
|---|---|---|
| `go build`, `go vet`, the built program started by a session | works | works; its `/bin/sh`, `/bin/ls`, `#!/bin/sh` and `/usr/bin/env` are redirected (`/usr/bin/env` is `EPERM` today) |
| `go run`, `go test`: the program calls a shell or tool **by name** | works | works |
| the same **by absolute path**, or a `#!/bin/sh` script | works (the host's files) | **`EPERM`** |
| `//go:generate sh -c ...` | works | works |
| `//go:generate ./gen.sh` (`#!/bin/sh`) | works | **`EPERM`** (generate stops there; `/bin/sh` as the command was not reached) |

The cause is the one already published: Apple's dyld drops the library in Go's
hardened binary, and its children inherit nothing.

**A remedy that needs no re-signing:** `go run` and `go test` start the built
program through `-exec`. With `GOFLAGS='-exec=env DYLD_INSERT_LIBRARIES=<lib>
DEVCROFT_NS=<ns>'` (the values a session already has) all four tests and
every `go run` case pass, and `go build` ignores it. It does not reach
`go generate`, whose commands Go starts itself.

## Found while qualifying (2026-11-24), beyond BR-01 to BR-10

All in stock `brush-shell` 0.4.0 too: an `IFS` that is not whitespace splits
literal words (`IFS=:; echo a:b` prints `a b`); its empty fields are lost;
`printf` with arguments its format does not use never ends (`printf a b`; first
seen through `IFS`, but it needs none); an `EXIT` trap in a subshell does not
run; a script on standard input is run a line at a time, so a multi-line `if`
fails (devcroft's adaptor reads complete commands, and gives brush an
unbuffered descriptor 0 so `read` does not take the rest). For the owner to
file with the others (`new/brush-issues-2026-11-24.md`). **Owner, 2026-11-24:** the `IFS` and `printf` defects are accepted and
published; they do not hold slice A back, and devcroft keeps no fork.

## Risks / Trade-offs

- [mise/pixi projects whose scripts rely on traps, `$!` or `wait PID` break when
  their `/bin/sh` becomes brush] → published per defect with the remedy
  (declare `bash`); `up` says so once for affected sandboxes.
- [Removing host `/bin` on macOS breaks `#!/bin/bash` scripts in mise/pixi
  projects that work today] → BREAKING, published, same remedy.
- [Upstream fixes take long] → slice A still delivers the invariant; slice B
  waits; a devcroft-maintained fork is a separate owner decision.
- [Two `uucore` versions in the tree] → measured; resolved when brush updates.
- [Interactive hangs on terminals without cursor reporting] → basic input
  fallback, proven by the test-row fixture.

## Migration Plan

- Slice A: affected mise/pixi sandboxes change on `up --recreate`; `up` prints
  the removed host grants once; a new runtime generation.
- Slice B: interactive sessions change on the next login after the generation
  that ships it; `$SHELL` keeps its precedence.

## Open Questions

- Whether slice B should apply only where the environment's shell is a
  non-interactive build, or to every non-native sandbox.
- Whether brush's history file stays `.brush_history` or follows the
  environment shell's `HISTFILE` convention.

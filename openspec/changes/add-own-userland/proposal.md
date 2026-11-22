# Change: add-own-userland

## Why

devcroft removes the host's userland from every sandbox (`own-policy-baseline`,
`add-mount-isolation`) but supplies none of its own. Whether `ls`, `cat` or
`env` exist in a session therefore depends on what the provider happened to
pull in: nix's stdenv brings GNU coreutils; a minimal flox manifest and a devenv
profile without declarations bring nothing (measured 2026-11-19); every one of
the eleven `devenv-<agent>-sample`s declares `coreutils findutils gnugrep
gnused` by hand for exactly this reason. Artifact providers fill the gap from
the host instead (mise, pixi), which breaks the owner's reproducibility goal.

The owner's direction (2026-11-22): **devcroft owns the absolute-path namespace
for shell and userland and fills it with what the sandbox resolves by name —
the provider's environment first, devcroft's runtime where the provider
supplies nothing — and the runtime is pinned by the project** (rule "B"). So
`/bin/ls` and `ls` always run the same executable: GNU coreutils from the
closure in a nix or devbox sandbox (both supply the full GNU userland, measured
2026-11-22), devcroft's runtime in a flox sandbox (which supplies none of 24
utilities, measured). This change delivers the runtime, its place on `PATH` on
both platforms, the absolute-path namespace on Linux, the project-side pin,
and the single declared native exception. `add-runtime-provenance` is the
witness it is measured against.

## What Changes

- **Base runtime, embedded.** uutils utilities are linked into the `devcroft`
  binary and dispatched on `argv[0]` (multicall), each in its own process
  through `uucore::bin!`. The published command surface does not change:
  `devcroft ls` stays an unknown command.
- **Generation, pinned by the project.** `devcroft.toml` gains
  `[runtime] generation`. A binary embeds exactly one generation; a mismatch is
  refused at layer `config`. A manifest without the key gets the current
  generation, and `up` prints it with the line to pin it.
- **`PATH` order.** Provider environment first, then the runtime, then nothing
  from the host. A project that declares its own `coreutils` keeps it.
- **Absolute-path namespace on Linux.** In every sandbox view, including fleet
  views, `/bin/<name>` and `/usr/bin/<name>` for each base name resolve to the
  same executable the sandbox resolves for `<name>` — the provider's if it
  supplies one, else the runtime's. `/usr/bin/env` is therefore **always**
  present. `/bin/sh` is the shell resolved for the sandbox. A base name neither
  supplies stays absent, and so does every absolute path outside the base
  names.
- **One namespace directory per sandbox.** `up` builds it once (one link per
  base name to its resolved target); the Linux view mounts it at `/bin` and
  `/usr/bin`, and `add-macos-exec-redirect` redirects into the same directory,
  so both platforms resolve from one computation.
- **One common runtime layer.** Applied after provider resolution for every
  provider, replacing the per-provider shell and userland fallbacks. A provider
  may declare **native** mode only with a reason that is shown; Swift is the
  only one that does. Ad-hoc grants of Apple tools are refused elsewhere.
- **mise and pixi** take utilities and `env` from the runtime. Their shell
  fallback stays the host's until `embed-brush-runtime` provides one, and is
  reported as `host:` (unchanged from today, now visible).
- **find, grep, sed.** uutils now publishes `findutils`, `diffutils`,
  `uu_grep` and `sed` on crates.io. Each is qualified against the options real
  workloads use and admitted or not by a recorded decision.
- **BREAKING (macOS, mise/pixi):** none in this change; the removal of host
  `/bin` grants happens with `add-macos-exec-redirect` and
  `embed-brush-runtime`.

## Capabilities

### New Capabilities
- (none new — `base-runtime` is introduced by `add-runtime-provenance`)

### Modified Capabilities
- `base-runtime`: adds the embedded runtime, generation pinning, `PATH` order,
  the Linux absolute-path namespace, the common runtime layer and the native
  declaration.
- `config`: adds the `[runtime]` table and its `generation` key.

## Impact

- `src/bin/devcroft.rs`: `argv[0]` dispatch before any parsing.
- New `src/runtime/` module: inventory, generation, materialization of the
  runtime directory, the common layer.
- `src/provider/mise.rs`, `src/provider/pixi.rs`, `src/provider/swift.rs`: drop
  the duplicated fallback; swift declares native.
- `src/lifecycle/up.rs` (`path_ending_in_shell_dir` generalised),
  `src/fleet/mount.rs` (`Identity` and view construction), `src/fleet/project.rs`.
- `src/config/mod.rs`: `[runtime]`.
- `Cargo.toml`: `uu_*` and `uucore` promoted to `[dependencies]`; packaging
  allowlist unchanged (no new `[[bin]]`).
- `THIRD-PARTY-LICENSES.md` regenerated; binary size grows (measured in the
  design's estimate at ≈ 14–15 MB stripped, to be confirmed).
- Samples: the `devenv-<agent>-sample`s can drop the declarations the runtime
  now covers; whether they do is a task, measured per sample.

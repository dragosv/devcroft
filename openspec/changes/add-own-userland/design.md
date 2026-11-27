## Context

Second of four changes carrying out
`new/devcroft-runtime-absolute-paths-design-2026-11-22.md`. The evidence it
builds on, restated so the change stands alone:

- **The gap is real and provider-shaped** (`userland-gap-survey-2026-11-19`,
  measured live): a flox session with a minimal manifest has no `ls`, `cp`,
  `cat`, `grep`, `sed`, `env`; a nix session has GNU coreutils through stdenv;
  devenv profiles carry only what is declared. The survey's ecosystem
  proportions were found unreliable (`adversarial-review-latest-analysis` R1,
  R2) and are **not** used here; the owner's reproducibility goal is the
  justification, the measured sessions the evidence.
- **The multicall model was prototyped** in
  `spikes/brush-uutils-native/` (on the owner's Linux host; not in this
  checkout) with 39 coreutils applets through `uucore::bin!`: 119 utility probes
  pass, `SIGPIPE` and binary output behave, `PATH` order "project before
  runtime" holds, an empty or replaced `PATH` gets no hidden fallback.
- **Costs measured for the scratch builds** (`userland-floor-decision`):
  39 applets at uutils 0.12 ≈ 9.95 MB stripped standalone; the baseline
  `devcroft` figure there was not tied to a source revision (R9), and the crate
  delta was a name diff, not an inventory delta (R10). Both are re-measured in
  this change's tasks on the integrated build.
- **What each environment supplies by name** (measured 2026-11-22 on macOS,
  inside real sandboxes, 25 names): nix (`nix-flake-sample`) and devbox
  (`devbox-citytime-sample`, and a postgres-only project without a toolchain)
  supply all of them — bash 5.3 and GNU coreutils 9.11/9.12, grep 3.12, sed 4.10,
  gawk 5.4.1, findutils 4.11, diffutils 3.12, tar 1.35, gzip, make 4.4.1. flox
  (`flox-services-sample`, and `flox-clap-sample` with a Rust toolchain)
  supplies **none** of 24 utilities; its shell is a bash devcroft takes from the
  closure's requisites. devenv was not measured (not installed on that host);
  its samples declare the userland by hand.
- **The owner chose rule B** (2026-11-22): an absolute path resolves as its name
  does. Under it the runtime's utilities are used in flox, devenv, mise and pixi
  sandboxes and wherever a project's environment lacks a name — not in nix or
  devbox sandboxes, whose GNU userland they would otherwise shadow by absolute
  path.
- **uutils publishes more than coreutils now** (verified 2026-11-22 with
  `cargo info`): `findutils` 0.11.0, `diffutils` 0.6.0, `uu_grep` 0.2.0,
  `sed` 0.3.0, all from `github.com/uutils`. `uu_ls` is at 0.13.0.

- **Spike evidence is not available in this repository** (task 0.2): the
  `spikes/brush-uutils-native/` tree lives only on the owner's Linux host, so
  every claim above that rests on it is re-measured here on the integrated
  build rather than cited.
- **Baseline, measured** (task 0.3; `37c605d`, aarch64 Linux, rustc stable,
  `cargo build --release`): `devcroft` 35,692,272 bytes, **25,555,840
  stripped**; **381 shipped dependencies** as the generator counts them over its
  four targets. (The generator's text hashes for a few crates differ between
  hosts — e.g. `deranged`, `num-conv` — while the count does not; a
  pre-existing nondeterminism in which license file it picks, not a change in
  what ships.)

- **Cost, measured** (task 6.1, 2026-11-23, `3cfbbba`, rustc 1.95.0): aarch64
  Linux `cargo build --release` 45,681,368 bytes, **33,356,376 stripped**,
  +7.8 MB (+31%) on the 0.3 baseline, which predates every change since
  `37c605d` and not only this one; 189 s clean with 6 jobs. Size-optimised
  (`opt-level = "z"`, fat LTO, one codegen unit, stripped): 16,947,600 bytes,
  167 s; the crate defines no such profile and no baseline exists for it.
  macOS arm64: 36,228,416, 29,683,168 stripped, 176 s. Shipped dependencies:
  **511** (381 before, +130). Earlier build times were not recorded.

## Goals / Non-Goals

**Goals:**
- A runtime devcroft owns, versioned as a generation the project pins.
- `PATH` filled from the runtime where the provider is silent, never from the host.
- The Linux absolute-path namespace (`/bin/<name>`, `/usr/bin/<name>`,
  `/usr/bin/env`) in every view, resolving exactly as the name does.
- One place that decides shell and userland fallback, and one declared native
  exception (Swift).

**Non-Goals:**
- A devcroft shell. `/bin/sh` stays the closure's; the runtime shell is
  `embed-brush-runtime`.
- macOS absolute paths. `add-macos-exec-redirect`.
- awk, tar, or anything without a published uutils crate.
- A security claim. Utilities are executables under the same boundary as
  everything else.

## Decisions

### D1. Multicall inside `devcroft`, one process per call
Linked `uu_*` crates, dispatched on `argv[0]`'s basename as the first thing in
`main`. Rejected alternatives, as recorded in `userland-floor-decision` §2: a
second `[[bin]]` (`cargo install` would put a generic binary on the user's host
`PATH`, the `spike` lesson); an `include_bytes!` blob written at `up` (bytes in
`devcroft` anyway, plus a write step and weaker provenance); calling `uumain`
in-process (it mutates the environment, calls `exit`, owns signals).
Each applet goes through the upstream wrapper of the pinned version
(`uucore::bin!` or its equivalent), never a bare `uumain` call: at 0.12 the
wrapper also initialises localisation, without which help and diagnostics print
translation keys (R5).

*Measured at 0.13.0 (2026-11-22):* `bin!` expands to a `pub fn main()` that
calls `$util::uumain`, so each applet gets a module of its own whose name is
not the crate's (`true`, `false`, `test` cannot name modules anyway). Its
locale data is embedded by `uucore`'s build script, which, for a crates.io
build, embeds the `.ftl` files of **every** `uu_*` crate unpacked beside it in
the cargo registry, plus the build host's `LANG` locale if one exists. The
English text the applets print does not depend on either, but the binary's
bytes do: two hosts with different registry contents or `LANG` build different
`devcroft` binaries from the same lock. Recorded, not fixed (upstream's build
script); it matters if devcroft ever claims reproducible binaries.

### D2. The generation is a small integer the project pins
`[runtime] generation = N`. It identifies behaviour, not the devcroft release:
it changes when the inventory, a utility version, or a mapping rule changes. A
binary embeds one generation and refuses a mismatch, the `rust-toolchain.toml`
model.
*Alternatives:* (a) report only, no pin — rejected, the owner's goal is
reproducibility per project, and today's closure shell is already pinned by the
project's lock, so a report-only runtime would weaken it (review N2);
(b) ship several generations in one binary — rejected for size and test
surface; (c) a content-addressed runtime artifact separate from the binary —
kept as a future option if refusal proves too strict in practice.

### D3. `PATH` = provider, then runtime, then nothing
Generalises `path_ending_in_shell_dir` (`src/lifecycle/up.rs:1726`), which
already appends the resolved shell's directory last so the project's choice
wins. The runtime directory is appended after it. Host directories that a
provider's captured `PATH` ends in (the canonical tail) are dropped from the
sandbox `PATH`; they are refused by policy anyway, and leaving them makes
`command -v` lie. The function is shared with `fleet::project::prepare`, which
today mirrors `up`'s rule (`src/fleet/project.rs`), so the two cannot drift.

### D4. One namespace directory, computed once, mounted on Linux
Rule B makes the namespace a function of the resolved environment: for each
base name, the target is the first hit for that name in the provider's `PATH`
(inside a granted path), then the shell directory, then the runtime. `up`
writes that function out once, as a namespace directory beside the runtime
(`<project>/.devcroft/<name>/ns/`), one symlink per base name that has a
target: a store path for what the environment supplies, a runtime link for the
rest. Nothing is resolved twice, so `/bin/ls` cannot disagree with `ls`.
On Linux the view mounts that directory's contents as a small tmpfs at `/bin`
and `/usr/bin` (store targets are already in the view; runtime targets point at
a single bind-mounted `devcroft`). `Identity` (`src/fleet/mount.rs:229–297`)
keeps the passwd, group and hosts files, and its `/bin/sh` and `/usr/bin/env`
entries become namespace entries. Origins: runtime entries `runtime:gen-N`;
entries whose target is in the closure are already covered by the provider's
grants and render with provenance `closure:`.
`add-macos-exec-redirect` redirects into the same directory, so the two
platforms share one computation and one test of it.
*Alternative (rule A, rejected by the owner 2026-11-22):* absolute paths always
to the runtime. It gives one implementation across projects but two inside a
nix or devbox sandbox (`ls` GNU, `/bin/ls` uutils; `sh` bash, `/bin/sh` brush).
The resolver must not canonicalise the runtime's `sh` link once
`embed-brush-runtime` uses it: a stored path to the `devcroft` binary loses the
basename the dispatch needs (handoff §14.3). Recorded here so the namespace
shape does not have to change later.
*Alternative:* bind each runtime file individually at its path. Rejected: N
bind mounts per view and per fleet agent, against one.

### D5. A common runtime layer after provider resolution
Providers report what their environment has (shell, `PATH`, grants). The layer
completes the namespace. mise and pixi lose their private fallback code
(`mise.rs:987–1012`, `pixi.rs:790–815`); until the runtime has a shell, the
layer supplies the host shell as one file grant with `host:` provenance and a
warning — the same behaviour as today on Linux, now in one place. macOS
`/bin` + `/var/select` grants stay until `add-macos-exec-redirect` and
`embed-brush-runtime` can replace them, and are reported.
Swift declares `native` with its reason (Xcode's `xcrun` runs `sh -c` and reads
`/var/select/sh`; the shim is `#!/bin/sh`; `codesign` is in `/usr/bin`); its
entries render `native:swift`. Apple-signed binaries strip `DYLD_*`, so a
"half native" Swift is not possible on macOS anyway.

### D6. Inventory growth by decision
The coreutils floor (39 names) ships first. `findutils`, `uu_grep` and `sed`
are qualified against the option corpus of the agent samples and the hooks of
the qualified providers (`grep -rE`, `grep -n`, `sed -i`, `sed -E`, `find
-name/-type/-exec`); each is admitted by a recorded decision, with a new
generation. Under rule B these matter exactly where the environment is silent
(flox, devenv, mise, pixi). Success measure: how many of the eleven
`devenv-<agent>-sample` declarations become unnecessary.

*Corpus, first half (task 5.1, 2026-11-23).* The provider hooks and
activation scripts of every sample, read from the repository: `flox-clap`,
`flox-rustup` (`[hook].on-activate`), `devenv-sample` and
`devenv-antigravity-sample` (`enterShell`), `pixi-sample` and
`pixi-go-sample` (`[activation]` scripts and the `make` targets its tasks
run), `mise-sample` (`[tasks]`), `devbox-services-sample` (`[hooks]
post_create`), and the three nix samples' `shellHook`s. Outside the
provider's own tools (`cargo`, `go`, `gradle`, `rustup`, `initdb`) and shell
builtins, they call: `mkdir -p` with several operands, `date -u +FORMAT`,
`seq 1 N`, `sleep 0.1`, `echo`, and `[`/`test` with `-e`, `-f`, `-n`. Every
one is in the coreutils floor. **No hook in the repository calls `grep`,
`sed`, `find`, `xargs` or `diff`**, so this half of the corpus gives no
evidence for admitting any family.

*The second half is not in the repository.* What the eleven agents run
through their shell tools is not recorded anywhere here, and recalling it
would be the unmeasured claim this change exists to avoid. It needs one run
per agent in its sample with commands logged, which needs each agent's own
credentials, so it is the owner's to collect. Until it exists, 5.2 has
nothing to qualify against, and the families stay out.

One sample does mention these names: `pixi-go-sample` lists `grep` and `sed`
as packages. Its `Makefile` only prints where each one is found (`command -v`)
and never runs them, so the conclusion stands.

*Collecting the second half (2026-11-27).* The macOS redirect library can
now log every exec, when `up` runs with `DEVCROFT_LOG_EXECS` set. The log is
`.devcroft/<name>/run/exec.log`, already writable from inside, so the policy
does not change. `scripts/exec-corpus.py` turns logs into a table of each
command name with its option tokens. The log does not depend on the agent:
it sits under every agent's shell tool, so eleven agents need one procedure,
not eleven log formats (`docs/agents.md`, "Recording what an agent runs").
Measured on a flox sandbox:

- A bare name searched by libSystem (`execvp`, `posix_spawnp`) used to log one
  line per `PATH` directory tried. It is logged once now, by name.
- A program that searches `PATH` itself (Python's `subprocess`, Node's libuv,
  Bun) used to log each miss. A miss is now recognised and skipped: `argv[0]`
  is a bare name, the path ends in it, and nothing exists at that path.
- A program named by its full path is logged even when it is missing.
- `env -i` keeps the log.
- Bash's own "command not found" is never an exec, so it is not in the log.

The log is macOS-only. The agents behave the same on both platforms, so a
corpus collected on macOS answers 5.2 for both.

## Risks / Trade-offs

- [Binary size roughly triples] → measured on the integrated build before
  acceptance; `release-small` profile evaluated; owner accepts or not, like the
  nono trust tail.
- [uutils ≠ GNU on some options] → only where the provider supplies nothing;
  published per utility; a project wanting GNU declares coreutils and wins.
- [Refusing a generation mismatch blocks a user after upgrading devcroft] →
  the error names both generations and the fix (pin the new one, or install
  the matching devcroft); the generation changes rarely by design.
- [Two `uucore` versions if brush's builtins pin an older one] → tolerated and
  measured; deduplicated when brush updates (`embed-brush-runtime`).
- [Attribution grows from ~380 entries] → regenerate on four targets; MIT
  entries without a vendored text fail the generator rather than pass.

## Migration Plan

- Existing manifests without `[runtime]` keep working and get a printed pin
  suggestion.
- Existing sandboxes get the runtime on `up --recreate`; a running keeper keeps
  its state until then.
- Samples: each `devenv-<agent>-sample` is re-measured without its manual
  userland declarations; the ones the runtime covers are dropped, the rest
  stay with a comment saying why.

## Open Questions

- Whether `generation` absent should become an error after one release
  (forcing a pin), or remain a printed suggestion.
- Whether to include `diffutils` at all; no qualified workload calls `diff` yet.

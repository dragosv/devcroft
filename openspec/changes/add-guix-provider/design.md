## Context

The Guix manual refused automated fetching on 2026-11-22 (HTTP 403), so the
documented facts below come from search excerpts of the manual and cookbook.
Everything else is marked as a hypothesis for group 0.

| Fact | Status | Source |
|---|---|---|
| `guix shell --search-paths` displays "the environment variable definitions that make up the environment" | documented | <https://guix.gnu.org/manual/en/html_node/Invoking-guix-shell.html> |
| Output is `export VAR="…"` lines; the cookbook wraps it in `eval` with `--pure` and `--root` | documented | <https://guix.gnu.org/manual/en/html_node/Search-Paths.html>, <https://guix.gnu.org/cookbook/en/html_node/Guix-environment-via-direnv.html> |
| `guix describe -f channels > channels.scm`, then `guix time-machine -C channels.scm -- …` replays the same Guix | documented | <https://guix.gnu.org/manual/en/html_node/Replicating-Guix.html> |
| Without pinned commits, `time-machine` follows `~/.config/guix/channels.scm`, `/etc/guix/channels.scm` or the defaults, at HEAD | documented | <https://guix.gnu.org/manual/devel/en/html_node/Invoking-guix-time_002dmachine.html> |
| `guix shell` auto-loads `manifest.scm`/`guix.scm` only from directories in `~/.config/guix/shell-authorized-directories` | documented | <https://guix.gnu.org/blog/2021/from-guix-environment-to-guix-shell/> |
| `guix shell` has no activation hook (no `shellHook` analogue) | hypothesis | — |
| Guix runs on Linux (x86_64, aarch64) and not on macOS | hypothesis, widely known | — |
| Store at `/gnu/store`, daemon socket at `/var/guix/daemon-socket/socket` | hypothesis | — |
| Recent Guix runs the daemon unprivileged | hypothesis | — |

## Goals / Non-Goals

**Goals:**
- A second closure-tier substrate with the same guarantee as nix: zero host
  library grants, host toolchain denied, measured by a real build.
- Capture that runs no project code beyond evaluation of the manifest and the
  pinned channels, which nix already has.

**Non-Goals:**
- macOS. There is no Guix there, and Guix in a Linux VM is the
  `add-macos-service-vm` direction, not this change.
- `guix home`, `guix system`, Shepherd services, and `guix pack`.
- `guix.scm` package definitions as the environment (`guix shell -D -f
  guix.scm`). This could follow later. The first slice is `manifest.scm`.
- Authenticating third-party channels on devcroft's behalf. Guix does that
  (channel introductions). devcroft only requires that the pin exists.

## Decisions

### G1. Both files, explicitly, and pinned

`up` passes `-C <channels.scm>` and `-m <manifest.scm>` explicitly, never
relying on auto-loading, so `shell-authorized-directories` never applies.
devcroft reads `channels.scm` and requires a `commit` for every channel. It
reads the file as data, never evaluating it itself. A `channels.scm` that is
not a literal list of `channel` forms (computed, with `load`, with
`(current-channels)`) is refused. *Alternative considered:* evaluating the
file with `guix repl` to normalize it. Rejected because it adds an evaluation
devcroft does not need just to read a pin.

### G2. Capture is a parse, never an eval

The `--search-paths` output is parsed as `export NAME="value"` lines with
the escaping Guix emits (0.2 records it). Any line outside that grammar fails
`up` at layer `provider`, quoting it. The values become the environment. No
shell evaluates the output. This is how `print-dev-env --json` is treated for
nix, minus the JSON.

### G3. Resolution host-side, with the daemon, stated

`time-machine` builds or substitutes the pinned Guix, evaluates the channels
and the manifest, and builds or substitutes the profile through the daemon.
That is repository-controlled Scheme evaluated host-side, the same category as
nix evaluating a flake. `sandbox-provisioning` P6/P6a measured that no
provider works without daemon authority at resolution, and `policy --render`
already states it. The Guix line names `manifest.scm` and `channels.scm`.

### G4. GC roots under devcroft's state

`--root=<state>/<name>/guix-profile` registers the profile as an indirect GC
root, so `guix gc` cannot collect a live sandbox's closure. `rm` removes the
link. `status` reports a missing root as broken, not stale.

### G5. Store root as provider data

`capture::DEFAULT_STORE_ROOT` (`/nix/store`) becomes a value each provider
supplies. Guix supplies `/gnu/store`, and `capture::store_grants` and
`src/shell.rs`'s closure fallback take it from the provider. This is the
"seam stays generic" claim of `docs/decisions.md` §1 put to its first
non-Nix closure test.

### G6. Mount view

The view exposes `/gnu/store` read-only and nothing under `/var/guix`. An
ungranted daemon socket then fails `ENOENT`, as the nix one does
(`tests/unix_socket_not_mediated.rs` gains a Guix row).

## Risks / Trade-offs

- [First `up` of a pin is slow, since `time-machine` may build Guix itself] →
  `doctor` and the first `up` say so; substitutes are honoured.
- [Few developers have Guix] → not scheduled until demand; tests self-skip
  through a capability probe, never `guix --version` (the CLAUDE.md rule).
- [`--search-paths` covers only search-path variables, not every variable a
  package expects] → 0.3 compares it with `guix shell --pure -- env` on the
  sample. A gap is either documented or fixed by using the profile's
  `etc/profile` as data.
- [Scheme manifests can compute anything at evaluation time] → as with nix,
  evaluation is resolution and runs host-side. G1 keeps devcroft's own
  reading of the pin literal.

## Open Questions

- Should a `guix.scm` (`-D -f`) environment be admitted in the first slice?
- Is `--root` honoured together with `time-machine`, or must the root be
  added in a second step?

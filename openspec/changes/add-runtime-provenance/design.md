## Context

This is the first of four changes that carry out the direction recorded in
`new/devcroft-runtime-absolute-paths-design-2026-11-22.md` (untracked at the
time of writing; the facts this change depends on are restated here):

> devcroft owns the absolute-path namespace for shell and userland and fills
> it with what the sandbox resolves by name: the provider's environment first,
> devcroft's runtime only where the provider supplies nothing (owner's choice
> "B", 2026-11-22). Every shell or utility a sandbox can execute comes from a
> pinned source — the provider's lock or a devcroft runtime pinned by the
> project — never from the host, except named exceptions. Swift is the only
> native exception.

Measured 2026-11-22 on macOS (by-name lookup inside real sandboxes): nix and
devbox environments supply bash and the full GNU userland (coreutils, grep,
sed, gawk, findutils, diffutils, tar, gzip, make), devbox also without a
toolchain; flox supplies none of 24 utilities, even with a Rust toolchain, and
its shell is a bash devcroft takes from the closure's requisites; devenv was not
measured (not installed), and its samples declare the userland by hand. Under
rule B the follow-up changes make `/bin/<name>` and `<name>` run the same
executable; this change's record is what lets a test assert that.

Before anything changes what `/bin/sh` or `ls` resolves to, the current state
must be visible and asserted, so that `add-own-userland`,
`add-macos-exec-redirect` and `embed-brush-runtime` each flip a measured
expectation rather than claim one.

Current state, as read from the code and measured on macOS 15.8.3 arm64
(2026-11-22):

| Provider | `/bin/sh` | `/usr/bin/env` | Other base names by absolute path | By `PATH` |
|---|---|---|---|---|
| nix, flox, devbox, devenv (Linux) | view → resolved closure shell (`Identity`) | view → closure `env` where the `PATH` has one inside a provider grant (flox: only since the fix below) | absent (not in the view) | closure, or absent |
| nix, flox, devbox, devenv (macOS) | `EPERM` (measured) | `EPERM` (measured, via shebang) | `EPERM` | closure, or absent |
| mise | environment shell, else host `/bin/sh` (file grant on Linux; `/bin` + `/var/select` on macOS) | host `/usr/bin/env` (granted) | on macOS the whole host `/bin` is reachable | tools, then the shell dir |
| pixi | prefix `sh`/`bash`, else host as for mise | not granted | as mise | prefix `bin`, then the shell dir |
| swift | host (declared, `provider:swift`) | host | host `/usr/bin` (declared) | toolchain, then canonical host `PATH` |

Measured on Linux 2026-11-22 (aarch64, OrbStack kernel 7.2.12; nix 2.31.5,
flox 1.14.0, devbox 0.18.0, devenv 2.2.2), every base name in both lookup
forms executed inside real sandboxes and compared with the record by file
identity (`-ef`) from inside the view: the provider rows of
`tests/base_runtime_provenance.rs`, plus `flox-services-sample` (flox without
coreutils or bash) and `devenv-sample` (devenv has no row), checked by hand
with the same probe. 0 mismatches on all five; no `host:` entry on any.

| Sandbox | Absent by name | Resolving by absolute path |
|---|---|---|
| nix row (bash, coreutils, python3, curl) | `hostname` | `/bin/sh`, `/usr/bin/env` |
| devbox row (same packages) | `hostname` | `/bin/sh`, `/usr/bin/env` |
| flox row (same packages) | `hostname grep sed awk find xargs diff tar gzip` | `/bin/sh` only (`/usr/bin/env` too after the fix below) |
| flox, `flox-services-sample` (python3, process-compose) | every name but `sh` | `/bin/sh` only |
| devenv, `devenv-sample` (ripgrep, jq) | `hostname` | `/bin/sh`, `/usr/bin/env` |

Three facts the macOS measurement could not show:

- **`/usr/bin/env` is absent in every flox sandbox, coreutils or not.**
  `sandbox_identity` (`lifecycle/up.rs`) takes an `env` only from a `PATH`
  entry that literally `starts_with` a provider grant, without
  canonicalizing; flox's `PATH` begins with `.flox/run/<env>/bin`, a symlink
  into `/nix/store`, so it never matches. `shell::resolve` canonicalizes
  both sides and so finds flox's shell. **Fixed in this change by owner's
  decision (2026-11-22)**, despite the no-resolution-changes non-goal: it
  is a bug against `Identity`'s own stated rule, not a policy change, and
  it adds no grant (the store is already granted). `shell::resolve_env_bin`
  now applies the shell's canonical guard; the flox row resolves
  `/usr/bin/env` and the provenance test fails with the old check.
- **On a merged-`/usr` host `sh` by name reaches the view's `/bin/sh`
  through `/usr/bin`.** The view recreates `/bin -> usr/bin` and writes the
  identity shell through it, so a `PATH` ending in the host's `/usr/bin`
  finds `/usr/bin/sh` = the closure shell. The first computation looked the
  same path up on the host, found `/usr/bin/dash`, and, because the identity
  rule's grant canonicalizes to that same host file, recorded
  `host:/usr/bin/dash` for flox-services-sample while the sandbox ran the
  closure's bash 5.3.9. The test then agreed, because it too canonicalized
  on the host. Both now resolve through the view (`ViewEntries::written_at`;
  `-ef` inside the sandbox).
- **The `#!/usr/bin/env` gap is measured, not inferred**: in
  flox-services-sample, `#!/usr/bin/env python3` fails `ENOENT` at spawn,
  `python3` by name runs, and a `#!/bin/sh` script runs.

mise and pixi were not measured on Linux (neither is installed on the
measuring host).

## Goals / Non-Goals

**Goals:**
- One record, computed once at `up`, of where every base name resolves, in both
  lookup forms.
- Rendered by `policy --render` and `status` from `Meta`, like everything else
  the policy shows.
- A test that cannot pass while the record lies, run on every provider row.
- The two unpublished gaps published.

**Non-Goals:**
- Changing any resolution. No grant is added or removed. One exception,
  a bug fix rather than a design change: flox's missing `/usr/bin/env`
  (Context).
- Refusing on host provenance. That becomes possible only once a runtime exists
  to fall back to (`add-own-userland`).
- A `native:` provenance. Swift's declared exception is introduced with the
  common runtime layer in `add-own-userland`; here Swift's entries are simply
  `host:`, which is the truth.

## Decisions

### D1. The base-name set is a constant with a revision, not configuration
The set is `sh`, `env`, and the 39 coreutils names of the measured floor
(`ls cat cp mv rm mkdir rmdir ln touch head tail wc sort uniq cut tr echo printf
test pwd env true false seq basename dirname readlink realpath stat chmod mktemp
tee date id uname hostname yes sleep nohup`), plus the userland families an
environment commonly supplies and scripts call by absolute path: `grep sed awk
find xargs diff tar gzip`. The set is about userland, not about what the runtime
ships: under rule B a name the runtime lacks still has a namespace entry when
the environment supplies it (`/bin/awk` in a nix sandbox). Extending the set is
a revision of the constant, recorded with the change that does it.
*Alternative:* derive the set from whatever the runtime ships. Rejected: the
witness must exist before the runtime does, and a set that grows silently with
the runtime cannot detect a name that stopped resolving.

### D2. Resolution is computed, not probed, and the test probes
`up` computes provenance from data it already has: the resolved environment's
`PATH` and grants, `ResolvedShell`, the view's `Identity` on Linux, and the
compiled policy's read/exec rules for absolute paths on macOS. It does not spawn
anything to find out. The provider-row test is what executes each name inside
the sandbox and compares.
*Alternative:* probe inside the sandbox at `up`. Rejected: it adds spawns to
every `up`, and a probe that runs inside the boundary cannot see why something
is absent (policy vs. not in the view), which is what the record must say.

### D3. Recorded in `Meta`, rendered from `Meta`
Same rule as the shell (`Meta.shell`): `policy --render` renders from `Meta`, so
a value the backend acted on but `Meta` lacks would break the "nothing the
render cannot show" invariant. Fleet agents compute it from their own view and
store it with their agent state.

### D4. `host:` warns once, never refuses, in this change
Refusing needs somewhere else to resolve to. Until `add-own-userland` exists,
the only thing a refusal could do is fail mise and pixi sandboxes that work
today. The warning lists every `host:` entry in one line group, per the
degraded-capabilities rule.

### D5. The test asserts the invariant only where it already holds
A test that fails on mise, pixi and macOS today would turn the suite red with no
fix in this change. So the test asserts two things: on every row, the record
matches execution; on rows that satisfy the invariant (closure providers on
Linux), no entry is `host:`. The rows that do not yet satisfy it carry their
expected `host:` entries explicitly in the test, so the follow-up change that
fixes them must update the expectation, visibly.

## Risks / Trade-offs

- [The computed record disagrees with reality in a way the test does not
  exercise, for example a grant that covers a path by a parent directory] →
  the test runs every name in both forms on every row; mismatches name the base
  name and form.
- [macOS `absent` hides the reason (policy refusal vs. missing file)] → the
  render shows the reason next to `absent` (`refused by policy` or
  `not present`), derived from the compiled rules.
- [The base-name list drifts from what scripts actually use] → the list is the
  measured floor; extending it is a recorded revision, and the agent samples'
  declarations are the yardstick (`add-own-userland`).

## Migration Plan

Additive. Existing sandboxes get the record on their next `up`. `status` on a
sandbox brought up by an older devcroft shows "provenance not recorded" rather
than computing it from a different source.

## Open Questions

- Whether `status` should exit non-zero when a `host:` entry exists under a
  future strict flag. Deferred to `add-own-userland`, which introduces the
  refusal path.

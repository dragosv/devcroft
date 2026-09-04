# Design — Nix-Free Test Row

## Context

This is `add-test-runtime-fixture`'s group 5, extracted because it turned
out to be a change with its own decisions — binary provenance, pinning,
licensing, and a per-platform split — rather than a task.

It starts from a better position than that group was written against. Its
first blocker is gone: `shell::resolve` no longer requires `/nix/store`, it
requires "inside a path the provider declared", and a row that declares its
own directory now satisfies it. Its second supposed blocker was never real —
`services::resolve_in_env` has no store check at all, only "must be on the
resolved environment's `PATH`".

**Four things below were measured on macOS 15.7.4 before this document was
written**, and they narrow the question from "can a Nix-free row exist" to
"where does its shell come from".

## Goals / Non-Goals

**Goals:**
- A row that needs no Nix daemon and satisfies the existing row contract.
- Enough of a real environment that the neutral surface genuinely runs on
  it — a shell that works, and `process-compose` where services are wanted.
- Honest per-platform scoping, stated up front rather than discovered.

**Non-Goals:**
- Not the default row; not evidence of closure-tier behaviour; not a
  replacement for the real-provider rows. See proposal Non-Goals.
- **Not a general-purpose shell of our own.** Writing a stub that implements
  enough of `sh -c` is a trap: the neutral tests use pipelines,
  redirection, `cd &&`, `while` loops. A stub either grows into a shell or
  quietly narrows what the suite can assert.

## Decisions

## N1 — The row declares its own directory as a grant (MEASURED, works)

**Decision.** The row's `Resolution` sets `PATH` to a directory it created
and lists that directory in `read_only_grants`. No store involvement.

**Measured.** Driving the real `up` through the `test-support` seam with
exactly that shape returned `Ok(Started)`, and `meta.json` recorded the
shell as `/private/tmp/…/bin/sh` — outside the store. This is the whole
mechanism, and it is already proven; what remains is content, not plumbing.

This works because `add-test-runtime-fixture` generalized the shell guard,
and it is the concrete payoff of that generalization.

## N2 — Copying a shell off the host is not an option on macOS (MEASURED, fails)

**Decision.** The row's binaries are never copied from macOS's own
`/bin`/`/usr/bin`.

**Rationale, and this is a measurement rather than a preference.** A copied
`/bin/sh` does not run: it **hangs indefinitely**. So does a copied
`/bin/echo`, so it is not specific to shells. The copies are byte-identical
in signature terms — `codesign -dv` reports the same `CodeDirectory v=20400
size=455` and the same signature size on original and copy — so a broken
signature is not the explanation. Whatever the mechanism (platform-binary
handling in the loader is the obvious suspect), the observable fact is
enough to decide with: it hangs rather than failing, which is the worst
shape for a fixture, because a hang looks like a slow test rather than a
broken row.

That closes the option independently of the taste argument. It was already
weak on taste — `test-runtime-fixture` requires that no row satisfy the
contract by resolving its shell from the host — but "it does not work" is a
shorter conversation than "it should not".

## N6 — A Rust shell as a dev-dependency, which supersedes N3 and N4 (MEASURED)

**Decision.** The row's shell is `brush` — a Rust implementation of a
POSIX/bash-compatible shell — taken as a **dev-dependency** and built by
devcroft's own `cargo build` through a one-line `examples/` wrapper.

This supersedes N3 (build dash from source on macOS) and N4 (static BusyBox
on Linux). Both stay below for the reasoning, which was sound for the
options known at the time; this one was not considered and is better on every
axis that mattered.

**Measured, all of it:**

| | value |
|---|---|
| crate | `brush-shell` 0.4.0 / `brush-core` 0.5.0, updated 2026-05-03 |
| licence | **MIT** |
| the wrapper | `fn main() { brush_shell::entry::run() }` — one line |
| behaviour | `-c`, `cd &&`, pipelines, `[ ]` tests, redirection: all work |
| new crates | **69** not already in devcroft's tree |
| binary | 34M debug (10.6M via `cargo install`) |

**What it dissolves, rather than solves:**

- **The platform split (N4) disappears.** One artifact, both platforms. There
  is no longer a Linux row and a macOS row that share no artifact and must be
  kept working separately.
- **Fetch-vs-vendor and hash pinning (tasks 3.1, 3.2) dissolve.** `Cargo.lock`
  already pins it, with the same discipline every other dependency gets.
- **The GPL attribution burden (task 3.3) disappears.** BusyBox is GPL-2.0 and
  the *binary* carries it. brush is MIT — and, decisively, the licence
  generator scopes itself to `cargo tree -e normal`, so a dev-dependency does
  not enter `THIRD-PARTY-LICENSES.md` at all. Verified in the script.
- **The macOS toolchain requirement (tasks 2.1-2.3) disappears.** No C
  compiler, no `configure`, no build caching question. Cargo already builds
  this project.
- **The `UE` hazard disappears.** Nothing is copied from a platform path, so
  nothing can land in an unkillable state.
- **The "no dynamic loader" limitation disappears too**, which was BusyBox's
  main weakness and is worth correcting rather than carrying forward. A Rust
  binary links the platform's C library dynamically: measured on macOS,
  `otool -L` on the row's shell shows `libSystem.B.dylib`, `CoreFoundation`,
  `IOKit` and `libiconv`. So the row *does* exercise the loader path a static
  BusyBox never would. (Expected to hold on Linux via glibc, but not verified
  from this machine.)

**Cost, stated plainly:** 69 crates in the dev tree. This project has recorded
objections at 141 (nono's trust tail) and 116 (nono-proxy), so the number is
not nothing — but both of those were *runtime* dependencies that ship, link,
and carry Apache-2.0 §4(a) obligations to recipients. This one ships nothing,
links nothing, and is invisible to the attribution file. Different calculus,
same care.

**Placement matters and is not obvious.** The wrapper goes in `examples/`,
not `src/bin/`. `src/bin/` targets are auto-discovered — CLAUDE.md records
that `!/src/bin/spike.rs` in the packaging allowlist is load-bearing for
exactly this reason — and they cannot use dev-dependencies, so a wrapper
there would force brush into the *shipped* dependency tree. Examples can use
dev-dependencies and are never installed.

**Alternative considered and rejected: nushell.** The obvious objection to
picking a 0.4.0 crate is that a far more mature Rust shell exists — nushell
is at 0.111.0, is embeddable as a library (`nu-engine`, `nu-protocol`), and
is by any normal measure the safer dependency. It is still the wrong one,
and maturity is not what decides it.

**nushell is not POSIX.** It is a structured-data language of its own.
Measured against the constructs this fixture actually runs (nushell 0.111.0):

| input | result |
|---|---|
| `echo NU-OK` | works |
| `printf "a\nb\n" \| wc -l` | works |
| `cd /tmp && pwd` | `Error: nu::parser::shell_andand` |
| `x=5; if [ "$x" -gt 3 ]; then echo GT; fi` | `Error: nu::parser::unknown_command` |
| `echo r > f && cat f` | `Error: nu::parser::shell_andand` |

`&&` is not its syntax (it has a dedicated error for it, because people
type POSIX at it), `[` is a list literal rather than `test`, and a POSIX
assignment followed by `then`/`fi` does not parse. There is no compatibility
mode in `--help`.

**Why that is disqualifying rather than inconvenient.** devcroft's shell is
not a shell *it* chooses — it is an interpreter for text *the project*
supplies. All four call sites pass `-c` with POSIX that came from somewhere
else: `hooks.rs` runs a flox `[hook].on-activate`, `services/mod.rs` hands
process-compose `shell_argument: "-c"` for each service command from the
manifest, plus SSH login sessions and `devcroft shell`'s fallback. A flox
hook writes `mkdir -p x && cd x`; `flox-services-sample` declares
`python3 -m http.server $API_PORT`. A row whose shell cannot parse that
cannot run what users run, so the neutral surface would silently narrow to
whatever the row happens to accept — the same trap that ruled out writing a
stub of our own, arriving from the other direction.

The comparison worth holding: an *incomplete* POSIX shell can be fixed or
swapped; a *mature non-POSIX* shell cannot run the input at all.

**Residual risk:** brush is a young implementation, so a compatibility gap
would surface as a mysterious test failure rather than a clear one. That is
real, and it is bounded by the row never being the default and by the
real-provider rows staying required. It is a genuine shell rather than a
stub, so the objection that ruled out writing our own does not apply.

## N3 — Freshly built binaries do run (MEASURED) — SUPERSEDED by N6

> Kept for the measurement, which stands: a freshly compiled binary runs from
> an arbitrary directory where a copied platform binary does not. The
> *decision* it justified — compile dash at fixture setup — is superseded by
> N6, which needs no C toolchain at all. dash 0.5.12 built in 6s, for the
> record, and is BSD-3-Clause apart from a GPL build-time generator
> (`mksignames.c`) that is not linked into the shipped binary.

**Decision (superseded).** On macOS, the row's shell is **compiled from
source** at fixture-setup time.

**Measured.** A trivial C program compiled with the host `clang` and run
from a scratch directory printed and exited 0. So the platform has no
objection to a binary living outside its usual place — only to a *copied
platform binary*.

**Trade-off, stated plainly:** this makes the macOS row depend on a C
toolchain at setup time. That is a host dependency, which is exactly what
devcroft refuses *for sandboxes* — but the row is not a sandbox, it is the
thing that builds one, and the alternative on this platform is no row at
all. The distinction to hold onto: the compiler is used to *produce* the
row's environment, and never enters the sandbox's own `PATH`.

**Alternative considered: vendor a prebuilt macOS shell.** Rejected for now
— it needs a signed, notarization-compatible binary, a pin per
architecture, and licence attribution in `THIRD-PARTY-LICENSES.md`, for a
row that is not the default. Reconsider if setup-time compilation proves
slow enough to matter.

## N4 — Linux is a different row, and says so — SUPERSEDED by N6

> Superseded: N6 uses one artifact on both platforms, so there is no longer a
> different row to describe. The cost this decision accepted — GPL-2.0
> attribution, per-architecture hash pinning, a fetch-or-vendor decision — is
> the cost N6 avoids entirely.

**Decision (superseded).** On Linux the row uses a **static BusyBox**, pinned
by hash per architecture. It is not the same artifact as macOS's, and the two are
documented as one row with two platform implementations rather than as a
uniform "test provider".

**Rationale.** Static BusyBox is the cheapest thing that is a real POSIX
shell on Linux, and it needs no toolchain at setup. Its cost is the one
named in the proposal's Non-Goals: no dynamic loader, so it exercises none
of the loader path the mount view has to support. That is acceptable for a
row that is explicitly not closure-tier evidence, and unacceptable as a
reason to weaken the real rows.

**Open**: whether BusyBox is fetched at setup (needs network, needs pinning)
or vendored (repo size, licence attribution). See Open Question 2.

## N5 — Services are opt-in for this row

`process-compose` has no store requirement — measured: a `process-compose`
in an ordinary non-store directory resolves fine, because
`services::resolve_in_env` only walks the resolved environment's `PATH`. So
the row *can* support services by shipping a binary, and `capabilities()`
tells the neutral tests whether this row's build did.

Keeping it opt-in rather than mandatory means the row stays cheap for the
lifecycle/exec/SSH band, which is most of the neutral surface, and pays for
process-compose only where a test needs it.

## Risks / Trade-offs

- **[Risk] The row becomes the default by convenience**, and the suite
  quietly stops exercising a real closure. → **Mitigation**: the default is
  decided in `add-test-runtime-fixture` and is the Nix row; this change adds
  a row, it does not touch selection. A test asserting the default row's
  identity would make that regression loud.
- **[Risk] macOS setup-time compilation is slow or fragile** (no Xcode CLT
  on a given host, for instance). → **Mitigation**: it is a row like any
  other — unavailable is a reported skip with a reason, and the reason names
  the missing toolchain.
- **[Trade-off] Two platform implementations mean two things to keep
  working**, and a change that fixes one can silently break the other. That
  is the honest cost of a platform with no static-linking story; the
  alternative was scoping the row to Linux and leaving macOS developers with
  no cheap row at all.
- **[Risk] Vendoring or fetching a binary drags in supply-chain
  obligations** this project takes seriously — `THIRD-PARTY-LICENSES.md` is
  generated and audited. → **Mitigation**: Open Question 2 decides fetch vs
  vendor *before* any binary lands, and either way the attribution step is a
  task rather than an afterthought.

## Migration Plan

The row lands unused, then `add-test-runtime-fixture` group 4 migrates the
neutral surface onto the matrix. Migrating before the row exists is what
turns daemon-less hosts red, which is the sequencing that change recorded.

## Open Questions

1. **~~Does the macOS row compile a shell?~~ ANSWERED — it does not need to.**
   N6 takes a Rust shell as a dev-dependency, so there is no host toolchain
   step on either platform. (dash was measured at 6s if that path is ever
   revisited.)
2. **~~BusyBox: fetched or vendored?~~ DISSOLVED by N6** — neither. Cargo.lock
   pins it, and a dev-dependency never reaches the published crate or the
   attribution file.
3. **Does the row actually run the neutral surface?** N1 proved `up`
   reaches `Started`; it did **not** prove a session runs, because the
   shell available at the time was the copied one that hangs. The first
   task is to redo that end-to-end with a shell that works — until then,
   "the row functions" is proven only up to `up`.

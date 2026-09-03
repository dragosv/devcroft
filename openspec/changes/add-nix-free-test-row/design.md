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

## N3 — Freshly built binaries do run (MEASURED), so macOS builds from source

**Decision.** On macOS, the row's shell is **compiled from source** at
fixture-setup time.

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

## N4 — Linux is a different row, and says so

**Decision.** On Linux the row uses a **static BusyBox**, pinned by hash per
architecture. It is not the same artifact as macOS's, and the two are
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

1. **Does the macOS row compile a shell, or is there a better source?**
   N3 says compile, on the strength of "fresh binaries run". Not yet
   measured: *which* shell (dash is the obvious candidate — small, POSIX,
   no dependencies), how long it takes, and whether it needs anything
   beyond the Xcode command-line tools this project already assumes.
2. **BusyBox: fetched or vendored?** Fetch needs network at setup and a
   per-architecture hash pin; vendor needs repo size and the same pin, plus
   a decision about `Cargo.toml`'s anchored `include` allowlist so it does
   not ship in the published crate. Both need licence attribution.
3. **Does the row actually run the neutral surface?** N1 proved `up`
   reaches `Started`; it did **not** prove a session runs, because the
   shell available at the time was the copied one that hangs. The first
   task is to redo that end-to-end with a shell that works — until then,
   "the row functions" is proven only up to `up`.

# Design — add-swift-provider

## Context

devcroft's three shipped providers (flox, nix, devbox) are all frontends
over one substrate: the Nix store. Adding a fourth of the same kind would
prove nothing. `swift` is different on both axes the project models —
guarantee tier and activation safety — which is what makes it expensive
and what makes it informative.

Everything below was measured on Swift 6.1.2 / macOS 15, arm64, on
2026-09-07. Linux is unmeasured and every claim says so where it matters.

## Decision 1: the tier becomes a value in the code, not a word in a doc

`docs/decisions.md` §1 says "the tier is always visible in `status` and
once at `up`". It was never implemented, because all three providers were
closure tier and the sentence had nothing to distinguish. `swift` is the
first artifact-tier provider, so the sentence acquires content.

`ProviderKind::tier()` returns a `Tier` (`Closure` | `Artifact`), exposed
on `ProviderEntry` so an injected test provider must answer it too. `up`
prints it once; `status` prints it every time.

**Rejected**: inferring the tier from whether `read_only_grants` contains
a store path. That is a heuristic standing in for a fact the provider
knows, and it would silently reclassify a closure provider whose store
moved.

## Decision 2: toolchain discovery is data; only the lockfile check runs code

Two separate questions, with two different answers, and conflating them
is how this provider would have been worse than it needs to be.

**Where is the toolchain** — answered by `swift -print-target-info`,
which emits JSON and evaluates no project file:

```json
"paths": {
  "runtimeLibraryPaths": ["/Library/Developer/CommandLineTools/usr/lib/swift/macosx",
                          "/usr/lib/swift"],
  "runtimeResourcePath": "/Library/Developer/CommandLineTools/usr/lib/swift"
}
```

Those paths, plus the directory of the resolved `swift` binary and (on
macOS) `xcrun --show-sdk-path`, are the `provider:swift` grants. Every
grant traces to a measured path rather than a guessed constant.

**Does this package need a lockfile** — cannot be answered without
running `Package.swift`, because the dependency list lives in the
program's output. This is the criterion-4 violation, and it is confined
to exactly one call (`swift package dump-package`) whose only consumed
output is the `dependencies` array.

**Rejected**: skipping the lockfile check to avoid the violation. It
would make `Package.resolved` unenforced, dropping criterion 2 as well
as 4 and leaving the provider with no reproducibility claim at all. If
project code is going to run, it should at least buy the lockfile
guarantee.

**Rejected**: requiring `Package.resolved` unconditionally. SwiftPM does
not create one for a package with no external dependencies, so this
would refuse valid projects — a rejection with no remedy.

## Decision 3: the violation is reported through the mechanism that exists

`Resolution::ran_activation_hook` already routes to a warning at `up`
naming the provider and telling the user to treat `up` on an unread
repository as running its code. `swift` sets it `true`, always — not
conditionally on whether the manifest looked dangerous, because the
provider cannot know that, and a warning that is sometimes suppressed is
one people learn to expect silence from.

This is the one place where the flox precedent applies cleanly: that
field exists because a provider might be unable to hand back an
environment without running the project's code. flox stopped needing it;
`swift` is what it was for.

## Decision 4: `Package.resolved` is the fingerprint's lock half

Staleness pairs `Package.swift` with `Package.resolved`, through
`capture::optional_file_part` so an absent lockfile and a present-empty
one stay distinct — the property that helper exists to preserve.

## Decision 5: no services, and it must be loud

Swift has no service concept. `ServiceSupport::Unsupported`, so a project
declaring `[services]` under this provider fails at `up` through
`ensure_no_services_declared_for_another_provider` rather than silently
starting nothing.

## What is deliberately not built

- **No toolchain materialization.** devcroft does not install Swift. The
  precondition is a `swift` on `PATH`, checked at `up` and cheap —
  criterion 6 is the one criterion this provider passes without argument.
- **No `swiftly` integration.** It is a single-ecosystem toolchain
  manager, rejected by §1's rustup entry on criteria 5 and 6; adopting it
  would add a second failing dependency to a provider that already has
  one.
- **No Linux verification.** The grant set is derived from
  `-print-target-info` on both platforms, so the mechanism is portable,
  but only macOS is measured. Stated in the code and in `known-gaps`.

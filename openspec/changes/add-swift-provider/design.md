# Design — Swift Provider (Xcode / Command Line Tools)

## Context

devcroft has three providers, all Nix-based, all closure tier. This change
adds a fourth that shares neither property: it is host-linked (artifact
tier) and macOS-only. Both are firsts, and both are the reason this
document exists — the previous two providers needed a design document to
record one activation mechanism each, whereas this one needs it to record
two decisions that constrain what devcroft may claim.

The `Provider` seam is already provider-agnostic and has been exercised by
three activation mechanisms at near-zero marginal cost. `Resolution`'s
`read_only_grants` field was documented from the start as the place an
artifact-tier provider would declare host library access. Nothing in the
seam has to change; what has to be decided is what the provider does
*not* do.

Measurements below were taken on macOS 15.7.4 / arm64, Command Line Tools
with Swift 6.1.2 and MacOSX.sdk 15.5 (build 24F74). This host has the
Command Line Tools and no `Xcode.app`, which is the more constrained of
the two supported layouts and therefore the right one to design against.

**Constraint that shapes everything else:** provisioning runs host-side,
before any restriction, with the invoking user's own network and
filesystem access. That phase is trusted because it runs pinned tooling
from a lockfile. Any provider that runs project code there voids the
reason it is trusted.

## Goals / Non-Goals

**Goals:**

- A Swift/Xcode project gets a devcroft sandbox without adopting Nix.
- Resolution executes **no project code**, so criterion 4 holds strictly
  rather than approximately.
- The artifact tier becomes visible in the compiled policy and in the CLI,
  not only in `docs/decisions.md`.
- The provider fails closed, loudly and specifically, on a platform that
  cannot back it.
- Zero new dependencies, zero new commands.

**Non-Goals:**

- **Resolving, fetching or building SwiftPM dependencies at `up`.** This
  is the central non-goal, not an omission. See D1.
- **Sharing dependencies between sandboxes.** SwiftPM has no
  content-addressed store; this change does not build one.
- **iOS, watchOS or tvOS SDKs, simulators, or `xcodebuild` project
  builds.** The provider resolves a macOS toolchain. Simulators are a
  daemon-mediated, host-global resource and belong to a separate decision.
- **Code signing.** Signing needs the login keychain, which is exactly the
  host secret the boundary exists to keep out. Out of scope by design, and
  to be stated as such rather than left to be discovered.
- **Pinning the toolchain.** devcroft resolves whichever toolchain the
  host has selected. Making Xcode versions reproducible is not something a
  sandbox can do from the outside.

## Decisions

### D1. Never evaluate `Package.swift` host-side — take criterion 4, pay criterion 3

**Decision:** provider resolution discovers a toolchain and stops. It does
not read the project's manifest, does not run `swift package resolve`,
does not populate `.build`. Dependency work happens inside the sandbox.

**Why, measured.** SwiftPM has no hook-free entry point. Every command
that yields a package graph — including `dump-package`, which looks like
the data-returning entry point the nix provider uses — compiles and runs
`Package.swift`. A manifest with a side effect, under
`swift package dump-package`:

```
warning: 'probe': MANIFEST-SIDE-EFFECT-RAN
```

The provider table in `docs/decisions.md` §1 gains a row with an empty
right-hand column, and unlike flox's it cannot be worked around by
deriving a hook-free copy: flox's hook is a stanza that can be stripped
while leaving a byte-identical locked package set, whereas `Package.swift`
*is* the manifest. There is nothing to strip.

**SwiftPM sandboxes the evaluation, and this was worth measuring rather
than trusting.** From inside a manifest under `dump-package`:

```
PROBE read-$HOME/.ssh:     ALLOWED
PROBE read-/etc/passwd:    ALLOWED
PROBE write-project-root:  denied
PROBE network-egress:      denied
PROBE exec-/bin/echo:      ALLOWED
```

It is a **write-and-network sandbox, not a read or exec sandbox**. So the
mitigation exists, is real, and does not cover the dimension that matters:
a hostile `Package.swift` evaluated at `up` reads anything the invoking
user can read. `~/.ssh`, cloud credentials, tokens. devcroft would be
adding nothing on top, because at that moment no devcroft restriction
exists.

**Alternative considered — resolve host-side and accept it, as flox
does.** Rejected on a difference of degree that becomes one of kind:
flox's `[hook].on-activate` is optional and most manifests omit it, so the
risk is opt-in; `Package.swift` is mandatory in every SwiftPM package, so
the risk would be universal. A warning that fires for every single project
is not a warning.

**Alternative considered — resolve host-side under an additional devcroft
sandbox.** Rejected for this change: it means applying a restriction
during the provisioning phase, which is `sandbox-provisioning`'s entire
subject. If that change lands, this decision should be revisited — a
confined host-side resolve would let criterion 3 be recovered, and this
paragraph is the marker for that.

**What it costs.** Criterion 3 fails. Eight Swift sandboxes cost eight
fetches and eight builds, where eight flox sandboxes cost one. The
proposal states this and the CLI must say it out loud (D4).

### D2. Grant the shared cache, not the dylib paths

**Decision:** `read_only_grants` names the developer directory and the
dynamic linker's shared cache. It never names individual system dylibs.

**Why, measured.** A trivial SwiftPM executable links three host
libraries:

```
/usr/lib/libSystem.B.dylib
/usr/lib/libc++.1.dylib
/usr/lib/swift/libswiftCore.dylib
```

None of the three exists as a file:

```
/usr/lib/libSystem.B.dylib          ABSENT (dyld shared cache)
/usr/lib/libc++.1.dylib             ABSENT (dyld shared cache)
/usr/lib/swift/libswiftCore.dylib   ABSENT (dyld shared cache)
```

The obvious implementation — read `otool -L` output, grant each path — is
wrong in the worst available way. macOS grants match paths as spelled
(`docs/known-gaps.md`), so those rules compile, appear in
`policy --render`, look correct, and enforce nothing. The failure is
invisible at every point a reviewer would look.

This is also why the `policy` delta requires compilation to surface a
grant naming an absent path rather than emit it silently. That guard is
general, not Swift-specific, and this provider is simply the first place
the mistake is reachable.

### D3. Redirect SwiftPM's caches into the project root

**Decision:** the provider sets SwiftPM's cache, scratch and configuration
locations into the project root, rather than granting the home directory.

**Why.** SwiftPM writes to three separate home-directory locations,
measured present on this host: `~/.swiftpm`,
`~/Library/Caches/org.swift.swiftpm`, and `~/Library/org.swift.swiftpm`.
All are baseline-denied. Granting write access to any of them hands
project code a write into the user's home directory for the lifetime of
the sandbox, and granting all three is close enough to granting `$HOME`
that the distinction stops being meaningful.

SwiftPM exposes `--cache-path` and `--scratch-path`, so the lever exists.
This mirrors the `GOTMPDIR` finding recorded for `nix-probe-sample`: the
provider's environment names a location the policy denies, and the fix is
the tool's own override rather than a widened policy.

**Open sub-question flagged for task group 1:** flags are not environment
variables, and devcroft injects an environment rather than wrapping
commands. If SwiftPM honours no environment variable for these paths, the
options are a `swift` shim on the sandbox `PATH`, or granting a
devcroft-owned directory. **This must be measured before the mount plan of
this provider is written**, in the same spirit as `add-mount-isolation`'s
task group 0. Do not assume an env var exists.

### D4. The tier is stated in terms of what does not hold

**Decision:** `up` prints one notice naming the artifact tier *and the
property that fails* — environments are not shared between sandboxes, and
runtime behaviour depends on host libraries. `status` shows the tier but
does not repeat the notice.

**Why.** The standing rule is that devcroft does not market two
guarantees under one word. A notice that says "artifact tier" and stops
satisfies the letter of that and none of its intent, because the tier name
means nothing to a first-time user. The existing degraded-capability
warning is the right shape to copy: aspect, reason, fallback.

The tier is read from recorded metadata, not derived separately at each
display site, for the same reason `policy --render` renders from `Meta` —
two derivations of one fact eventually disagree.

### D5. Platform gating is its own error, not an existing category

**Decision:** a new provider-error variant for platform mismatch. `swift`
on Linux is not "unknown", "not yet supported", "out of scope by design",
or "a version manager".

**Why.** Each existing category would tell the user something false. The
provider exists, is in scope, is not a version manager, and is known. What
is missing is the platform. Reusing the nearest category would degrade
`validate.rs`'s central property — that a rejection names the specific
thing that fails.

**Rejected:** silently resolving a Linux Swift toolchain. Swift on Linux
is a real toolchain, but it is not Xcode-backed, has a different SDK
model, and links a different C library. One provider name resolving two
materially different environments by platform is the ambiguity the
guarantee-tier rule exists to prevent.

### D6. One spelling, no aliases

`nix` has aliases because `flake`/`flakes` name the same thing. `swift`
gets none. `swiftpm` would name the half D1 deliberately excludes, and
`xcode` would name one of the two backing installations — a project whose
manifest says `xcode` would be wrong on a colleague's machine that has
only the Command Line Tools, which is the layout this design was measured
against.

## Risks / Trade-offs

- **Criterion 3 genuinely fails, and this change ships anyway.** →
  Mitigation is disclosure, not engineering: the tier notice at `up`, an
  entry in `docs/decisions.md` §1, and an entry in `docs/known-gaps.md`.
  If the maintainer judges that a provider failing a qualification
  criterion should not ship at all, that is a coherent position and this
  change should be rejected rather than amended — §1 already says the
  honest response to the artifact tier proving too fine a distinction is
  to reject the tier outright.
- **This is the only provider that runs exclusively on the backend that
  does not mediate exec.** Seatbelt applies `(allow process-exec*)`. For
  closure-tier providers the practical mitigation is that the policy
  denies the host toolchain's paths; here the host toolchain is granted by
  design, so that mitigation is thinner. → No mitigation is offered.
  `add-backend-capabilities` exists to make this machine-readable, and
  should land first if the two compete.
- **The toolchain is not pinned.** Two developers on different Xcode
  versions get different environments from an identical manifest. → State
  the SDK build version in `status`, so the difference is visible rather
  than mysterious. Actually pinning it is out of scope (Non-Goals).
- **Dependency fetching now needs a `[network]` allowlist entry.** A user
  moving from bare `swift build` to devcroft will hit a refusal on their
  first build. → The refusal must be attributable to the declared policy,
  not presented as a provider error, and `init` should say so for Swift
  projects. This is the same behaviour change flox hooks saw under
  `own-policy-baseline`, and it was right there too.
- **`.build` is per-sandbox and large.** → Accepted; it lives in the
  project root, which is already read-write.

## Open Questions

1. **Does SwiftPM honour environment variables for cache and scratch
   paths, or only flags?** D3 depends on it. Must be measured before
   implementation, not assumed.
2. **Should `up` refuse a project whose `Package.resolved` is absent while
   `Package.swift` declares dependencies?** The devbox provider refuses
   when a declared package has no lockfile key, on the principle that
   nothing resolves at `up`. Here nothing resolves at `up` by
   construction, so the check would be about the *sandbox's* build
   succeeding rather than about provisioning — a different justification,
   and possibly not devcroft's business. Reading `Package.resolved` is
   safe (it is JSON, not code); reading `Package.swift` to learn whether
   dependencies are declared is not.
3. **Does the artifact tier warrant a `samples/` convention of its own?**
   Every existing sample demonstrates a closure-tier property. A sample
   whose point is a guarantee that does *not* hold is a new kind, and
   `nix-probe-sample` is the nearest precedent.

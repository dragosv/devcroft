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

**Measured, task 0.1 — the lever exists for one half and not the other.**

| state | default location | env var | flag |
|---|---|---|---|
| scratch (`.build`) | project root | **`SWIFTPM_BUILD_DIR` works** | `--scratch-path` |
| cache (repositories, manifests) | `~/Library/Caches/org.swift.swiftpm` | **none found** | `--cache-path` works |

`--cache-path` was verified to redirect completely: after a build with it,
the real home cache's mtime was byte-identical to before, and the
redirected directory held the `repositories/` and `manifests/` trees. But
no environment variable for it exists — `strings` over `swift-package`
yields `SWIFTPM_BUILD_DIR` and no cache equivalent.

**So the fallback in this decision is now the live path, not a
contingency**, and D3 resolves to: `SWIFTPM_BUILD_DIR` for scratch, and
for the cache either a `swift` shim on the sandbox `PATH` that appends
`--cache-path`, or a devcroft-owned granted directory. Task 1.3 chooses
between those two; both are strictly more work than the env var this
design originally assumed.

This mirrors the `GOTMPDIR` finding recorded for `nix-probe-sample`: the
provider's environment names a location the policy denies, and the fix is
the tool's own override rather than a widened policy.

**A measurement hazard found the hard way, recorded so it is not repeated.**
The first probe of this question set `HOME` to an empty directory and ran a
build, including one with a real remote dependency. Nothing appeared under
that directory, and the obvious reading — "SwiftPM needs no home-directory
access" — is **wrong**. The real home cache had been written the whole
time: `~/Library/Caches/org.swift.swiftpm/repositories/swift-argument-parser-*`
carried the build's own timestamp. macOS resolves the home directory from
the password database, not from `$HOME`, so **`HOME=` is not a valid way to
test home-directory writes on macOS**, and any test in task group 6 that
tries it will pass while measuring nothing. Compare mtimes, or run as a
different user.

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

1. ~~**Does SwiftPM honour environment variables for cache and scratch
   paths, or only flags?**~~ **Resolved, task 0.1**: scratch yes
   (`SWIFTPM_BUILD_DIR`), cache no. See D3 for what that costs.
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

## Task group 0 results

Recorded here rather than lost in a terminal, since three of these change
what the implementation has to do.

**0.1 — resolved.** See D3.

**0.5 — resolved.** Every unhealthy state exits 1 and is distinguishable
by its message, so task 3.3 can tell them apart without guessing:

| state | exit | first line |
|---|---|---|
| stale selection (`DEVELOPER_DIR` absent) | 1 | `xcrun: error: missing DEVELOPER_DIR path: …` |
| CLT-only host, Xcode-only tool | 1 | `xcode-select: error: tool 'xcodebuild' requires Xcode, but active developer directory … is a command line tools instance` |
| healthy | 0 | the answer |

The second row is the more useful one, and it is the presence-versus-capability
rule in miniature: `/usr/bin/xcodebuild` **exists** on a Command Line Tools
host, so `command -v xcodebuild` succeeds and running it fails. Task 3.1's
probe must execute, exactly as `provider::host_can_build_nix_closures` does.

**0.2, 0.3, 0.4 — not resolved, and all three are blocked on the same
thing.** This host has the Command Line Tools and no `Xcode.app`, so the
second layout cannot be measured here at all:

- **0.2** — the CLT half is partly known (build writes go to the project's
  `.build` and to the home cache; reads come from the developer directory),
  but a full file trace needs `fs_usage`, which needs root. Not taken.
- **0.4** — CLT layout confirmed: `xcode-select -p` gives
  `/Library/Developer/CommandLineTools`, with `usr/bin/swift` and
  `SDKs/MacOSX.sdk` beneath it. The Xcode layout's
  `Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk` is confirmed
  **absent** here, which is evidence the two layouts differ but not
  evidence of what the second one is.
- **0.3** — the path is now certain: the shared cache lives at
  `/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld/`
  (`dyld_shared_cache_arm64e`), and `/System/Library/dyld/` **does not
  exist** on macOS 15. `DYLD_PRINT_LIBRARIES` confirms the mechanism —
  the binary loads `/usr/lib/libSystem.B.dylib` and a dozen
  `/usr/lib/system/*.dylib` that are all absent from the filesystem.

  What could not be confirmed is the runtime half of D2, and the reason is
  worth recording: **a `(deny default)` Seatbelt profile makes the process
  hang rather than fail.** `sandbox-exec` with `(allow default)` runs the
  binary and exits 0; the same binary under a deny-default profile granting
  only the shared cache blocked indefinitely, surviving both a `kill -9`
  watchdog and a `perl alarm` exec wrapper. A probe that cannot distinguish
  "denied" from "stuck" measures nothing, so this is deferred to task 6.4,
  which runs under devcroft's own compiled policy rather than a hand-written
  profile. Anyone tempted to re-attempt it with `sandbox-exec` should expect
  the hang, not a denial message.

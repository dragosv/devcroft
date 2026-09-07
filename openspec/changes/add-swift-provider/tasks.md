# Tasks — add-swift-provider

## 0. Measure before building

- [x] 0.1 Establish whether `Package.swift` is data or code, and whether any
      entry point returns the package graph without evaluating it.
      → **Code, and no.** SwiftPM compiles it with `swiftc` and runs the
      resulting binary; a deliberately invalid manifest reports
      `Invalid manifest (compiled with: [".../swiftc", …, "Package.swift",
      "-o", "…/probe-manifest"])`. `dump-package`, `resolve`, `describe` and
      `build` all evaluate it. No counterpart to `print-dev-env --json` or
      `shellenv --pure` exists.
- [x] 0.2 Establish what SwiftPM's own manifest sandbox does and does not
      block, since "it is sandboxed" would change how bad 0.1 is.
      → **It blocks writes, not reads**, which is the finding that settled
      the design. A write to `/tmp` from manifest code was refused; a read of
      `~/.ssh` succeeded and left the host's key filenames in the package data:
      `"name": "LEAK[HOME=/Users/dragos][SSH=id_ed25519,known_hosts.old,…]"`,
      from a plain `dump-package` with the sandbox on. No Seatbelt on Linux
      means no manifest sandbox there at all.
      Worth stating precisely because the first probe *looked* clean: the
      write side effect did not fire, which reads as "sandboxed, fine" until
      you test the other direction.
- [x] 0.3 Establish a data-only source for the toolchain paths the grants
      need, so the provider does not have to guess constants.
      → `swift -print-target-info` returns JSON with `runtimeLibraryPaths` and
      `runtimeResourcePath`, evaluating no project file. This is what let the
      provider separate "where is the toolchain" (data) from "does this
      package need a lockfile" (code), instead of paying the execution cost
      for both.

## 1. Tier becomes a value

- [x] 1.1 `Tier` enum (`Closure` | `Artifact`) with `ProviderKind::tier()`.
      → `docs/decisions.md` §1 has required the tier to be user-visible since
      before any provider existed; it was never implemented because all three
      providers were `Closure` and the sentence had nothing to distinguish.
      Rejected the shortcut of inferring the tier from whether
      `read_only_grants` holds a store path: a heuristic standing in for a
      fact the provider knows, and one that would silently reclassify a
      closure provider whose store moved.
- [x] 1.2 Put it on `ProviderEntry` so an injected provider must answer too.
      → Both test rows (`NixFreeRow`, `BorrowedStoreRow`) now answer
      `Tier::Closure`. They are behind `feature = "test-support"`, so a plain
      `cargo build --all-targets` compiled without them and *looked* like the
      trait change cost nothing — checked with the feature on rather than
      trusting that.
- [x] 1.3 Surface at `up` once and in `status` always; the artifact line
      names the host-linked cost rather than printing a bare word.
      → Printed for **every** provider, closure included. A line that appeared
      only for the weaker tier would make its absence the signal, which is the
      thing that goes unnoticed. Asserted in the e2e test on both `up` and
      `status`, and unit-asserted that the artifact string contains "host".

## 2. The provider

- [x] 2.1 `src/provider/swift.rs` with preconditions in a fixed order.
- [x] 2.2 Toolchain discovery from `swift -print-target-info` (data only).
- [x] 2.3 Grants, every one traced to a measured path.
      → Runtime library paths, the runtime resource path, the SDK
      (`xcrun --show-sdk-path`), the developer directory, **and the directory
      holding the host's POSIX shell**. That last one was not in the plan and
      is the most interesting: `up` failed with "no POSIX shell found in this
      environment or its closure" because `src/shell.rs` accepts a shell only
      inside a declared grant, and its doc comment says exactly why — "a host
      shell is still refused, because no provider declares a grant containing
      one." For the closure providers that is right. A Swift toolchain is not
      a closure and ships no shell, so the host-linked provider must declare
      the host path, which is what the artifact tier *means*.
      `add-test-runtime-fixture` anticipated this case when it generalized
      that guard from a literal `/nix/store`; `swift` is it arriving.
- [x] 2.4 Lockfile precondition, required only when dependencies exist.
      → SwiftPM writes no `Package.resolved` for a dependency-free package, so
      requiring it unconditionally would refuse valid projects. Both
      directions asserted in one test, because either half alone is satisfied
      by a wrong implementation ("always require" passes the first, "never
      require" the second).
- [x] 2.5 `ran_activation_hook: true`, unconditionally.
- [x] 2.6 `ServiceSupport::Unsupported`.
- [x] 2.7 Fingerprint over `Package.swift` + `Package.resolved`.
- [x] 2.8 Register in `ProviderKind`, dispatch, and `validate` — no aliases.

## 3. Tests

- [x] 3.1 Unit: target-info parsing, including shapes it must reject.
      → A `paths` block without `runtimeResourcePath` fails rather than
      yielding an empty string: an empty resource path would compile a grant
      list that looks populated and produce a sandbox that cannot build.
- [x] 3.2 Unit: the lockfile rule both ways.
- [x] 3.3 Unit: tier values for all four providers, stated per provider
      rather than as a rule a wrong implementation would also satisfy.
- [x] 3.4 Validation: `swift` accepted, `swiftpm`/`spm` refused.
- [x] 3.5 e2e: `tests/swift_provider_e2e.rs`. Asserts `up` succeeds, the tier
      appears in `up` **and** `status` naming its cost, the execution
      disclosure fires naming the provider, `policy --render` shows
      `provider:swift` origins, and the resolved environment reached the
      sandbox. A failed `up` here is a failure, not a skip — every
      precondition is checked first.
      **It deliberately does not assert that `swift build` succeeds inside**,
      because on macOS it does not; see 5.2.
- [x] 3.6 The control: `a_closure_provider_makes_no_such_disclosure`.
      → Uses `nix`, whose hook-freedom is *structural* (`print-dev-env --json`
      returns the `shellHook` as inert data), so a disclosure there would be a
      real regression rather than a fixture artifact.

## 4. Sample

- [x] 4.1 `samples/swift-spm-sample`, dependency-free — which is also the
      fixture for the conditional lockfile rule, and needs no network. Its
      README states the tier, the execution gap, and the macOS build
      limitation rather than showing commands that do not work.

## 5. Say what changed

- [x] 5.1 `docs/decisions.md` §1: filed under "Shipped despite failing the
      test", not under a rejection, so the six criteria keep meaning
      something. Names the two failing criteria, carries the leak
      measurement, and records the option not taken (serve Swift through the
      closure tier, as the rustup entry already does for Rust).
- [x] 5.2 `docs/known-gaps.md`: **two** entries, not one.
      The planned one: provisioning executes project code under this
      provider, with the read-exfiltration measurement and the Linux caveat.
      The unplanned one, found by running the sample: **`swift build` cannot
      run inside a sandbox on macOS.** The driver derives its cache path from
      `_CS_DARWIN_USER_TEMP_DIR` rather than `TMPDIR`, so injecting `TMPDIR`
      does not move it, and the path cannot be granted — `/var` is a symlink
      to `/private/var` and a grant does not cover the symlinked spelling,
      the pre-existing published defect. Confirmed it is not this provider's
      doing: devcroft's own baseline grants `/var/db/dyld` in both spellings
      and `ls /var/db/dyld` is still refused inside a sandbox.
      Two dead ends recorded rather than quietly dropped: granting both
      spellings of the temp dir (renders correctly, enforces nothing) and
      `XCRUN_NO_CACHE=1` (no effect — the error is the Swift driver's, not
      xcrun's).
- [x] 5.3 README: Swift named as the fourth provider and as the exception,
      with both costs stated where the provider list is, not in a footnote.
      `why --env` added to the command surface at the same time — it was
      missing there from `own-sandbox-environment`.
- [x] 5.4 `openspec/config.yaml` context: swift recorded as the artifact-tier
      provider that exists *and* fails the test. The pre-existing "no
      artifact-tier provider is scheduled" reasoning is kept rather than
      deleted, rescoped to every other candidate, since it still governs them.
- [x] 5.5 `init` detects `Package.swift` and selects `swift`, **ranked below
      all three closure providers**.
      → Reversed during review, at the owner's direction. The first
      implementation mentioned the provider but kept writing `flox`, on the
      reasoning that a generated manifest naming the one test-failing provider
      opts a user into a weaker guarantee they did not choose. The objection
      that carried: for a project with *only* a `Package.swift`, the
      alternative to selecting `swift` is not a better provider — it is a
      `devcroft.toml` naming `flox` for a project that has no flox
      environment, which fails on the first `up`. `init`'s job is to produce
      a manifest that works.
      What survives from the original concern is the **ordering**, which is
      where the care actually belongs: `flox`, then `devbox`, then a bare
      flake, then `swift`. A Swift package sitting beside any real closure
      environment keeps the stronger guarantee. Asserted for all three, in
      `init_prefers_every_closure_provider_over_a_swift_package` — without it,
      "swift ranks last" is an unverified claim in a comment.
      And the disclosure got *louder* rather than quieter, since this branch
      now reports a trade rather than a discovery: it names the artifact tier,
      names that `up` will run `Package.swift`, and names `flox init` as the
      way to avoid both. Both lines are asserted.

## 6. Scope the provider to projects nothing else can serve

> Raised in review after the provider was working: if a Swift package can
> build on Linux, flox or nix should have it — so `swift` should be refused
> there rather than merely available. The provider's justification in §1 is
> "Swift users otherwise get nothing", and that is only true for packages
> that need Apple frameworks. This narrows the provider to exactly that
> claim.

- [x] 6.1 Refuse `swift` for a package showing no Apple-platform dependency,
      at layer `provider`, naming `nix`/`flox` as the alternative.
      → New `ProviderError::CoveredByQualifiedProvider`. Distinct from every
      other rejection, which are about the provider alone: this one is about
      the provider **and this project together**, so the same name is correct
      one directory over.
- [x] 6.2 Accept only on positive evidence: a linked Apple framework, or an
      unguarded import of an Apple-only module.
      → Frameworks come from the `dump-package` call that already runs;
      imports come from a source scan that executes nothing.
- [x] 6.3 **Two traps that would have made the gate useless, in opposite
      directions.** Both measured, both asserted.
      `platforms: [.macOS(.v13)]` is **not** evidence — it sets minimum
      versions for Apple platforms and SwiftPM ignores it on Linux, so
      thousands of portable packages declare it; accepting on it would narrow
      nothing. And `Foundation`/`Dispatch` are **not** Apple-only — both ship
      on Linux via swift-corelibs, and counting them would qualify essentially
      every Swift package.
- [x] 6.4 A guarded import (`#if canImport(AppKit)`) is not evidence: that is
      a *portable* package with an Apple branch, which is the case the closure
      tier should get. Nesting tracked properly, so an `#if DEBUG` inside an
      `#if canImport` does not end the guard early — asserted, because the
      naive single-flag version gets this wrong.
- [x] 6.5 `.build/` excluded from the scan: a dependency's `import AppKit`
      says nothing about what *this* package needs, and without the exclusion
      every project that had ever run `swift build` would qualify.
- [x] 6.6 `init` applies the same gate, using the scan only.
      → It must not generate a manifest `up` then refuses. It deliberately
      does **not** run `dump-package`: `init` runs on a repository the user
      may have just cloned and has no business executing its code. The two
      checks therefore disagree in one direction — a framework-linking package
      with no Apple import gets `flox` from `init` and would be accepted by
      `up` — which is the safe direction, since `init` suggests the *stronger*
      provider.
- [x] 6.7 The sample had to change: it was portable, so the gate correctly
      refused it. `samples/swift-spm-sample` now uses an unguarded
      `import AppKit` and a real `NSWorkspace` call, with its README stating
      that swapping it for `Foundation` makes `up` refuse — the gate working,
      not a bug. The e2e fixture changed for the same reason, and gained a
      refusal test asserting exit 3 and the named alternative.
- [x] 6.8 Record that this is a heuristic and which way it fails.
      → Criterion 6 is hostile to heuristics, so the asymmetry is stated
      rather than glossed: a wrong refusal sends someone to a *better*
      provider and names what was searched for; a wrong acceptance silently
      downgrades their guarantee and runs their code on the host. Only the
      second is invisible to the user.

- [x] 6.9 **Widen the evidence to the deliverable, not only the source.**
      Raised in review: "so a Mac desktop project is mandatory? that is the
      target in principle." The gate never required *desktop* — the module
      list carries non-GUI frameworks (`Security`, `IOKit`,
      `ServiceManagement`) — but it did require the **source** to touch Apple,
      and that is a real gap: a Mac application whose Swift is entirely
      `Foundation` still cannot be produced by a Linux closure, because the
      app bundle, entitlements, code signature and `xcodebuild` are Apple-side.
      It was being refused and sent to flox, which can do none of those — a
      wrong refusal with **no remedy**, which is worse than the wrong
      acceptance the gate exists to prevent.
      Apple project artifacts are now evidence: `Info.plist`,
      `*.entitlements`, `.xcodeproj`/`.xcworkspace`, `.xcassets`,
      `.storyboard`, `.xib`, `.xcconfig`, `PrivacyInfo.xcprivacy`. All are
      filesystem checks — no parsing, no execution — so `init` runs them too.
      `.build/` stays excluded for artifacts as well as imports, or one
      dependency's `Info.plist` would qualify every project.
- [x] 6.10 Read `-framework X` from `unsafeFlags`, not only `.linkedFramework`.
      → Found while answering 6.9 and fixed regardless of it: `.linkedFramework`
      is the tidy spelling, not the only one, and packages reaching a non-GUI
      Apple framework often use the unsafe-flags form. Missing it refused real
      Mac projects. A dangling `-framework` with no name after it is ignored
      rather than recorded as an empty framework.

## 7. Reconcile with the prior analysis on `add-swift-provider`

> Found only when the Swift work was being moved to its own branch:
> `origin/add-swift-provider` already carried an analysis from 3-4 Sept,
> with independent measurements of the same facts and a **different, better
> conclusion**. Reconciled here rather than either branch silently winning.

- [x] 7.1 Adopt the correct framing: the provider is **Xcode/CLT-backed, not
      SwiftPM-backed**. Everything below follows from that one correction.
- [x] 7.2 Correct the criteria verdicts. This work had claimed failures on 4
      and 5; both were wrong.
      → **5 passes, unusually well**: the CLT tree ships clang, the linker,
      the macOS SDK, system headers and the Swift runtime — it is *the* C
      toolchain on macOS, not one ecosystem's slice. **4 passes, and more
      cleanly than any other provider.** The real failure is **3**, which this
      work had not evaluated at all: SwiftPM has no content-addressed shared
      store, so eight Swift sandboxes cost eight builds.
- [x] 7.3 **Stop evaluating `Package.swift`.** The violation this change had
      disclosed so carefully was avoidable, and the fix was to do less.
      → `dump-package` is gone. It existed to learn whether the package
      declared dependencies, so a lockfile could be required — but devcroft
      materializes no dependencies host-side, so it never needed the package
      graph. Removing it retires the execution *and* the lockfile precondition
      together, and turns `ran_activation_hook` from `true` to `false`.
      The e2e assertion is inverted rather than deleted: the disclosure's
      **absence** is now the property, so a future change that starts reading
      the package graph again fails a test instead of quietly giving up the
      position.
- [x] 7.4 Keep the framework signal without the execution: read
      `.linkedFramework("X")` and `-framework` out of `Package.swift` **as
      text**. Bounded and stated — a framework name computed during manifest
      evaluation is missed, which is a false negative in a gate whose false
      negatives send the user to a better provider.
- [x] 7.5 Add platform gating: `swift` fails closed off macOS. Swift exists on
      Linux; an Xcode-backed provider does not, and resolving silently under
      the same name would make one manifest mean two guarantees.
- [x] 7.6 Adopt the measured scratch lever. `SWIFTPM_BUILD_DIR` is honoured
      for the scratch directory (verified by moving `.build` out of the
      project); **nothing** is honoured for the cache — only `--cache-path`,
      which devcroft cannot use because it injects an environment rather than
      wrapping commands. Recorded as a gap rather than papered over.
      Also recorded: **macOS resolves the home directory from the password
      database, not `$HOME`**, so testing cache behaviour by redirecting
      `HOME` measures nothing. That one produced a confident wrong answer
      before it was caught.
- [x] 7.7 Republish the docs against the corrected analysis: `decisions.md`
      §1 rewritten around criterion 3, the "provisioning executes project
      code" gap **removed from `known-gaps.md` because it is no longer true**,
      and the no-shared-store cost published in its place. The macOS build gap
      no longer claims Linux as a fallback, since the provider refuses Linux.

## 8. What is left open

- [x] 8.1 **`swift build` now works inside the sandbox.** Closed by fixing the
      defect it was blocked on, which turned out to be devcroft's own and
      general rather than Swift-specific.
      → `policy::capability_set` canonicalized every grant before handing it to
      nono, so on macOS — where `/tmp` and `/var` are symlinks — only the
      canonical spelling ever got a rule. nono keys its macOS dedup on
      `original` precisely so both survive; devcroft had removed the
      distinction one layer earlier. Grants now emit the literal spelling too
      when it differs, additively, so nothing that worked before can regress.
      This is the fix `docs/known-gaps.md` already named ("emitting both there
      too — devcroft-side when it builds the grant list"), implemented rather
      than newly diagnosed. **I described it to the user as a misdiagnosis
      before reading the entry's body; that was wrong, and the entry was
      right.**
      Two measurements kept: **Seatbelt does resolve symlinks when matching**
      (a `deny` on the canonical path refuses the symlinked spelling too), so
      the failure was never literal matching — it was that the symlink
      component itself was ungranted, and `ls -ld /var` returned
      `Operation not permitted`. And **`sandbox-exec` cannot probe this**: a
      `(deny default)` profile hangs the process rather than failing it,
      surviving `kill -9`, so the instrument has to be devcroft's own policy.
      Three further requirements found by the build failing, now in the
      sample's manifest and in `known-gaps`: the two Darwin per-user
      directories granted read-write (no environment lever exists for either,
      and the provider must not grant them itself — outside the project root,
      write access, so it is the project's declaration to make);
      `swift build --disable-sandbox`, because Seatbelt does not nest and
      devcroft's sandbox is already the stronger one; and `/usr/share` +
      `/var/db/timezone`, without which every formatted date is silently
      empty.
      Verified live: `Build complete!`, and `swift run` prints the AppKit
      query from inside the sandbox.
- [ ] 8.2 Reconcile the two branches' remaining artifacts. This branch now
      carries the corrected analysis and a working implementation;
      `origin/add-swift-provider` carries a `policy` delta spec, a `doctor`
      arm, and a dyld-shared-cache grant finding this branch has not adopted —
      **`/usr/lib/libSystem.B.dylib` and friends do not exist as files** and
      are served from the shared cache at
      `/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld/`, so a grant
      naming them grants nothing and fails silently. Worth taking before this
      is called done.

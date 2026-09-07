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
- [x] 5.5 `init` mentions `Package.swift` but **never auto-selects** `swift`,
      unlike the three closure providers it does select. A generated manifest
      naming it would opt a user into a weaker guarantee and into host-side
      execution of their own repository without either being a decision they
      made. The advice names both costs and points at `flox init` for a
      reproducible Swift environment.

## 6. What is left open

- [ ] 6.1 **Linux is unmeasured**, and it is the platform where this provider
      is most likely to fully work: no `/var/folders`, no `xcrun`, no
      `/var` → `/private/var` symlink. Until someone runs it there, the
      honest claim is "brings a sandbox up on macOS, does not build in it".

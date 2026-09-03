# Change: add-swift-provider

Status: proposed (post-MVP). Depends on: `own-policy-baseline` (complete),
`add-devbox-provider` (implemented). This is the **first artifact-tier
provider** and the **first platform-specific provider** — both are new
concepts for the `Provider` seam, which is why it is worth more scrutiny
than provider #4 would otherwise get.

Every measurement quoted below was taken on macOS 15.7.4 / arm64 with
Command Line Tools 16 (Swift 6.1.2, MacOSX.sdk 15.5 / 24F74), not read
from documentation.

## Why

devcroft runs on macOS and has no macOS-native provider. All three shipped
providers are Nix-based, which on a Mac means a developer must adopt Nix
before devcroft has anything to offer them — while the toolchain they
already have, Xcode or the Command Line Tools, is by some distance the
most complete and most immutable thing on the machine.

Today `provider = "swift"` does not even get a considered answer. It falls
past every arm of `provider::validate` to `ProviderError::Unknown`, which
tells the user the name is unrecognised. That is the one rejection
category the framing rules say must not be reachable for a real candidate:
`docs/decisions.md` §1 distinguishes "out of scope by design", "fails the
qualification test", and "not yet supported", and Swift is none of those
because nobody has judged it.

This change judges it. **The answer is not a clean pass**, and the
proposal is written so the maintainer can reject it on the evidence rather
than adopt it on the enthusiasm.

## The qualification test, measured

Against `docs/decisions.md` §1's six criteria:

| # | Criterion | Verdict |
|---|---|---|
| 1 | Declarative manifest | **Pass, with a caveat** — `Package.swift` is a file, but it is *executable Swift*, compiled and run to produce the package description. See criterion 4. |
| 2 | Restorable lockfile | **Pass** — `Package.resolved` pins dependency revisions by commit SHA and `swift package resolve` reinstalls from it. |
| 3 | Immutable-capable shared store | **Fail** — see below. |
| 4 | Capturable activation without executing project code | **Pass, by design choice** — see below. |
| 5 | Completeness | **Pass, unusually well** — the CLT tree ships clang, the linker, the macOS SDK, system headers and the Swift runtime. It is *the* C toolchain on macOS, not one ecosystem's slice. Weakness: it is pinned by whatever the host installed, not by hash. |
| 6 | Verifiable preconditions | **Pass** — `xcode-select -p`, `xcrun --sdk macosx --show-sdk-build-version`, `swift --version` and the license-acceptance state are all cheap and checkable at `up`. |

### Criteria 3 and 4 are in direct tension, and that is the finding

This is the whole change in one paragraph. For Swift you can satisfy
criterion 3 or criterion 4, **not both**, and which one you take is a
decision about the user's secrets.

**Materializing dependencies requires evaluating the manifest.** SwiftPM
has no `print-dev-env --json` equivalent: there is no entry point that
hands back a resolved package graph without compiling and running
`Package.swift`. Measured — a `Package.swift` carrying a side effect,
under `swift package dump-package`:

```
warning: 'swiftprobe': MANIFEST-SIDE-EFFECT-RAN
```

It runs. That is the same class of problem `fix-provisioning-hooks` found
in flox, nix and devbox, except it is worse in one respect: flox's
`[hook].on-activate` is an opt-in stanza most manifests omit, whereas
**every** SwiftPM package has a `Package.swift`, so the code path is
universal rather than exceptional.

**SwiftPM sandboxes that evaluation itself — but not in the dimension
that matters here.** This was worth measuring rather than assuming, and
the result cuts both ways. Probing from inside a manifest under
`swift package dump-package`:

```
PROBE read-$HOME/.ssh:     ALLOWED
PROBE read-/etc/passwd:    ALLOWED
PROBE write-project-root:  denied
PROBE network-egress:      denied
PROBE exec-/bin/echo:      ALLOWED
```

So it is a **write-and-network sandbox, not a read or exec sandbox**.
A hostile `Package.swift` evaluated host-side at `up` runs with the
invoking user's full read access — `~/.ssh`, `~/.aws/credentials`, shell
history, any token on the disk — and may spawn host binaries. Provisioning
runs before any devcroft restriction exists, so devcroft would be adding
nothing on top. That is precisely the trust basis the two-phase execution
invariant rests on, and it does not survive contact with this provider.

**Therefore this change takes criterion 4 and pays criterion 3.** devcroft's
Swift provider **never evaluates `Package.swift` host-side, at all**. It
resolves the *toolchain* — `DEVELOPER_DIR`, `SDKROOT`, the toolchain's
`PATH` entry — which comes from `xcode-select` and `xcrun` with zero
project code involved, and is the cleanest criterion-4 pass of any
provider devcroft has. Dependency resolution moves *inside* the sandbox,
where a hostile manifest is confined by the policy the project declared.

The cost is real and must be published, not buried: **SwiftPM has no
content-addressed shared store**, so each sandbox resolves and builds its
own `.build/checkouts`. "Eight sandboxes cost one build" — the closure
tier's headline property — is false here. Eight Swift sandboxes cost eight
fetches and eight builds. Criterion 3 fails, and this change does not
pretend otherwise.

### What that makes it

**Artifact tier**, and the first one devcroft ships. The tier's defining
property is visible in the linkage of anything it builds — measured on a
trivial SwiftPM executable:

```
/usr/lib/libSystem.B.dylib
/usr/lib/libc++.1.dylib
/usr/lib/swift/libswiftCore.dylib
```

Host libraries, not toolchain-bundled ones. Per `own-policy-baseline` the
baseline grants none of those, so this provider must declare them itself
as `provider:swift` grants — which is exactly what `Resolution`'s
`read_only_grants` doc comment already predicted an artifact-tier provider
would have to do. This change is the first time that prediction is tested
against a real provider instead of a hypothetical one.

## What Changes

- **A `swift` provider** (`src/provider/swift.rs`) implementing the
  existing `Provider` trait. Resolution is toolchain discovery only:
  `xcode-select -p` for `DEVELOPER_DIR`, `xcrun --sdk macosx
  --show-sdk-path` for `SDKROOT`, the toolchain `usr/bin` prepended to
  `PATH`. No project file is read, opened or evaluated.
- **`swift` accepted by `provider::validate`**, moving it out of
  `ProviderError::Unknown`. `swiftpm` is *not* accepted as an alias —
  the provider resolves a toolchain, and naming it after the package
  manager would describe the half this change deliberately does not do.
- **Artifact tier becomes real in the code**, not just in
  `docs/decisions.md`. The tier is surfaced at `up` and in `status`, per
  the standing rule that devcroft does not market two guarantees under one
  word. A user running `swift` must be told, once, that they are not
  getting the closure-tier guarantee.
- **Platform gating.** `provider = "swift"` on Linux fails closed at layer
  `provider` with exit code 3, naming the platform. Swift exists on Linux;
  an Xcode-backed provider does not, and silently resolving a different
  toolchain under the same provider name would be the worst available
  outcome.
- **`doctor` learns the Swift preconditions** — CLT-or-Xcode present,
  which one is selected, SDK build version, license accepted — alongside
  the existing per-provider `doctor` arms.
- **Cache redirection.** SwiftPM writes to three `$HOME` locations
  (`~/.swiftpm`, `~/Library/Caches/org.swift.swiftpm`,
  `~/Library/org.swift.swiftpm`), all baseline-denied. The provider sets
  `--cache-path`/`--scratch-path`-equivalent state into the project root
  rather than granting `$HOME`, mirroring the `GOTMPDIR` lever found in
  `nix-probe-sample`.
- **`samples/swift-clt-sample`** — a runnable SwiftPM project, and the
  place the manifest-sandbox probe above is *measured* rather than
  asserted, in the shape `nix-probe-sample` established.
- **`docs/decisions.md` §1 gains a Swift entry**, and
  `docs/known-gaps.md` gains the no-shared-store consequence.

## Capabilities

### New Capabilities

None. This change deliberately introduces no new capability: the
`Provider` seam, tier vocabulary and policy-origin machinery all exist.
Needing a new capability here would itself be evidence the seam did not
generalize.

### Modified Capabilities

- `env-provider`: a fourth provider, the first that is artifact tier and
  the first that is platform-gated; the rule that a provider must not
  widen the policy now has a provider that genuinely needs host grants.
- `config`: `env.provider = "swift"` accepted and validated; rejected with
  a platform reason off macOS.
- `policy`: `provider:swift` host-library grants, including the dyld
  shared cache path (see Impact).
- `cli`: `status` and `up` surface the guarantee tier; `doctor` gains the
  Swift arm.

## Impact

- **Code**: `src/provider/swift.rs` (new), `src/provider/mod.rs` dispatch,
  `src/provider/validate.rs`, `src/bin/devcroft.rs` (`doctor`, `status`,
  the tier warning), `src/policy` for the tier-attributed grants.
- **A non-obvious implementation fact, measured, that will otherwise cost
  someone a day**: the host dylibs the linkage above names **do not exist
  as files**. `/usr/lib/libSystem.B.dylib`, `/usr/lib/libc++.1.dylib` and
  `/usr/lib/swift/libswiftCore.dylib` are each absent from the filesystem
  and served from the dyld shared cache. A `read_only_grants` entry naming
  them grants nothing, and — because Seatbelt matches paths as spelled
  (`docs/known-gaps.md`) — would fail silently rather than loudly. The
  grant has to name the shared cache.
- **Dependencies**: none added. No new crate, no new binary requirement
  beyond the Command Line Tools the host already has.
- **Scale**: the CLT tree is 2.1 GB and read-only; granting it is cheap in
  bytes because it is shared, and it is the one part of this provider that
  *does* satisfy "materialize once, expose read-only to many".
- **A tension worth recording rather than resolving here**: this is the
  only provider that runs exclusively on the backend that does not mediate
  exec. macOS Seatbelt applies `(allow process-exec*)`, so any host binary
  runs inside the sandbox whatever the policy says. For the closure-tier
  providers that gap is mitigated in practice by the policy denying the
  host toolchain's *paths*; for a provider whose entire premise is a
  granted host toolchain, the mitigation is thinner. This does not block
  the change — the same gap already applies to every macOS sandbox
  devcroft runs — but it is the reason `add-backend-capabilities` should
  land first if the two ever compete for the same release.

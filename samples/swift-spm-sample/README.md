# swift-spm-sample

A minimal SwiftPM executable, used as devcroft's `env.provider = "swift"`
sample.

## Why this sample imports AppKit

devcroft's `swift` provider **refuses a package that a closure-tier
provider could serve**. A portable Swift package builds fine from nix or
flox, where it gets reproducibility, a hook-free activation, and no
host-side execution of `Package.swift` — so offering it the weaker
provider would buy nothing and cost all three.

The provider is only justified where no closure can serve the project, so
this sample has to be genuinely macOS-only. The unguarded `import AppKit`
in `Sources/citytime/main.swift` is that: it cannot compile on Linux.

Replace it with `import Foundation` and `devcroft up` will refuse the
sandbox and point you at `provider = "nix"` or `provider = "flox"`. That
is the gate working, not a bug.

## What this sample demonstrates

- The `swift` provider resolving the host's Xcode / Command Line Tools
  toolchain at `up`, without opening any file in this package.
- The **artifact** tier: `devcroft up` and `devcroft status` both print
  the guarantee, and it is not the same guarantee the flox, nix and
  devbox samples get.
- Every host path the toolchain needs rendered with a `provider:swift`
  origin in `devcroft policy --render` — the artifact tier's cost made
  visible in the compiled policy rather than described in prose.

## What this sample does *not* demonstrate, deliberately

**Reproducibility.** This is the one provider devcroft ships that fails
the qualification test in `docs/decisions.md` §1. The Swift toolchain
comes from the host, so two machines with different toolchains produce
different behavior from the same `Package.swift` — that is what the
artifact tier means, and why `up` says so.

**A shared store.** SwiftPM has no content-addressed store, so eight
sandboxes of this project cost eight fetches and eight builds — not the
one build the flox, nix and devbox samples get. That is criterion 3 of
`docs/decisions.md` §1 failing, and it is what this provider pays in
exchange for never evaluating `Package.swift`.

## What it does *not* cost: your secrets

Worth stating because the obvious implementation gets it wrong.
`Package.swift` is a Swift program — SwiftPM compiles and runs it to
produce the package description, and its own sandbox around that
evaluation allows reads and exec, denying only writes and network. A
provider that called `swift package dump-package` at `up` would run this
repository's code on your host with your full read access.

devcroft does not. It resolves the *toolchain* (`xcode-select`, `xcrun`)
and opens no project file at all; dependency resolution happens inside the
sandbox when you run `swift build`. `up` on a Swift repository you have
not read does not execute it.

## Running it

```sh
devcroft up
devcroft policy --render     # every host path shows a provider:swift origin
devcroft status              # names the artifact tier
```

## Building it

```sh
devcroft up
devcroft exec -- swift build
devcroft exec -- swift run citytime
```

Plain `swift build`, and the manifest grants the project root and nothing
else. Two things make that true, and `up` names both:

**The `swift` on the sandbox's `PATH` is a shim** at
`.devcroft/swift/bin/swift`, a twelve-line shell script the provider
writes at every `up`. For `build`, `run`, `test` and `package` it adds
`--disable-sandbox` — SwiftPM sandboxes its own manifest evaluation with
`sandbox-exec`, Seatbelt does not nest, and inside devcroft that fails
with `sandbox_apply: Operation not permitted`; devcroft's sandbox is the
outer and stricter one, so nothing is lost — and `--cache-path`, since
SwiftPM's package cache defaults to `~/Library/Caches` with `~` taken
from the password database, not `$HOME`. Every other invocation reaches
the toolchain's own `swift` unchanged. Open the file; it is meant to be
read.

**Every cache and scratch directory is pointed inside the project** by
the environment the provider resolves: `SWIFTPM_BUILD_DIR` (`.build/`),
`TMPDIR`, `CLANG_MODULE_CACHE_PATH` and `xcrun_db` (all under
`.devcroft/swift/`). An earlier version of this sample granted two
`/var/folders/…` directories read-write instead — the Darwin per-user
scratch and cache — on the reasoning that no variable moved them. Two
did, once found: clang's module cache has `CLANG_MODULE_CACHE_PATH`,
and `xcrun`'s lookup cache has `xcrun_db`, an undocumented variable
read out of `libxcrun.dylib`. Those grants were exactly the widening the
artifact tier exists to avoid, and they are gone.

What the toolchain needs from the host beyond the project — the Xcode
bundle or Command Line Tools, the SDK, Foundation's ICU and time-zone
data, the license record, the host shell and userland — is the
provider's to declare: `devcroft policy --render` lists each with a
`provider:swift` origin. `devcroft doctor` checks the same preconditions
before `up`, the Xcode license included.

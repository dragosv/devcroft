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

## `swift build` does not work inside the sandbox on macOS

Stated plainly because it is measured, and because a sample that quietly
does not do what it shows is worse than one that says so.

`devcroft up` works: the environment resolves, the policy compiles, and
`DEVELOPER_DIR`, `SDKROOT` and `TMPDIR` are injected so nothing inside has
to look them up. The **build** then fails:

```
swift: error: couldn't create cache file
  '/var/folders/__/…/T/xcrun_db-…' (errno=Operation not permitted)
```

The Swift driver derives that path from `_CS_DARWIN_USER_TEMP_DIR`, not
from `TMPDIR`, so injecting `TMPDIR` does not move it — and the path
cannot be granted, because `/var` is a symlink to `/private/var` and **a
grant does not cover the symlinked spelling of its own path on macOS**.
That is a pre-existing published defect, not something this provider
introduced: devcroft's own baseline grants `/var/db/dyld` in both
spellings and `ls /var/db/dyld` is still refused inside a sandbox.
Granting the canonical `/private/var/folders/…/T` does not help either,
since the toolchain opens the `/var/…` spelling.

See `docs/known-gaps.md`. **Linux is unmeasured**, and has neither
`/var/folders` nor `xcrun`, so it is the more likely platform for this to
work on.

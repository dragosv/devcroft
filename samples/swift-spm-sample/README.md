# swift-spm-sample

A minimal SwiftPM executable, used as devcroft's `env.provider = "swift"`
sample.

## What this sample demonstrates

- The `swift` provider resolving a real package host-side at `up`.
- The **artifact** tier: `devcroft up` and `devcroft status` both print
  the guarantee, and it is not the same guarantee the flox, nix and
  devbox samples get.
- The conditional lockfile rule: this package declares no dependencies,
  so it has no `Package.resolved`, and devcroft accepts that. Add a
  dependency without running `swift package resolve` and `up` refuses,
  naming that command.

## What this sample does *not* demonstrate, deliberately

**Reproducibility.** This is the one provider devcroft ships that fails
the qualification test in `docs/decisions.md` §1. The Swift toolchain
comes from the host, so two machines with different toolchains produce
different behavior from the same `Package.swift` — that is what the
artifact tier means, and why `up` says so.

**A safe `up`.** `Package.swift` is a Swift program. SwiftPM compiles and
runs it to produce the package description, and devcroft must do that to
know whether a lockfile is required. `devcroft up` therefore prints a
warning naming this provider, and you should treat `up` on a Swift
repository you have not read as running its code. See
`docs/known-gaps.md`.

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

# Change: add-swift-provider

Status: proposed (post-MVP). Depends on: `own-policy-baseline` (complete),
`add-devbox-provider` (complete).

**This is devcroft's first artifact-tier provider, and its first provider
that fails the qualification test in `docs/decisions.md` §1.** Both facts
are the point of the change rather than side effects, and both are
adopted deliberately by the project owner over the objection recorded
below.

## Why

Swift is the largest ecosystem devcroft cannot serve at all. A Swift
project has no `flake.nix`, no `devbox.json` and no `.flox/`; it has
`Package.swift` and a toolchain installed by Xcode, the Swift.org
tarball, or a distro package. Those users get `provider::validate`'s
`Unknown` rejection today.

Swift also has a genuine lockfile — `Package.resolved` pins every
dependency to a revision with a checksum — so criteria 2 (restorable
lockfile) and 3 (immutable-capable shared store, via the SwiftPM clone
cache) are met more cleanly than for most rejected candidates.

## Why this is nevertheless a rejection on the project's own test

Measured on this host (Swift 6.1.2, macOS), not argued:

**Criterion 4 (capturable activation without executing project code)
fails, and there is no hook-free entry point.** `Package.swift` is not a
manifest; it is a Swift program. SwiftPM compiles it with `swiftc` and
runs the resulting binary to obtain the package description:

```
error: 'swiftprobe': Invalid manifest (compiled with: [".../swiftc", …,
  "Package.swift", "-o", "/var/folders/…/swiftprobe-manifest"])
```

Every entry point that yields the package graph — `dump-package`,
`resolve`, `describe`, `build` — evaluates it. There is no `--json`
data-only path as with nix's `print-dev-env`, and no `--pure` variant as
with devbox's `shellenv`.

**The confinement is weaker than it appears.** SwiftPM sandboxes manifest
evaluation on macOS, which blocks *writes* — and does not block *reads*.
Manifest code exfiltrates host state through the package data itself:

```
"name" : "LEAK[HOME=/Users/dragos][SSH=id_ed25519,known_hosts.old,…]"
```

That is a `swift package dump-package` on an unmodified toolchain, with
the sandbox on. On Linux there is no Seatbelt and no manifest sandbox at
all.

This is a **harder** failure than flox's, which `fix-provisioning-hooks`
resolved: flox's `[hook].on-activate` is *separable*, so devcroft
materializes from a derived hook-free copy and proves the package set
byte-identical. `Package.swift`'s code **is** the manifest. Strip it and
there is no package. The flox remedy is structurally unavailable.

**Criterion 5 (completeness) fails too.** SwiftPM resolves Swift package
dependencies. It does not deliver the toolchain, the C toolchain, the
SDK, or libc — those come from the host, which is the property that
rejected rustup.

## What is built anyway, and under what terms

The provider is implemented in full, at the **artifact** tier, with the
violation surfaced rather than hidden:

- Resolution reports `ran_activation_hook: true`, which routes into the
  warning `up` already prints for exactly this situation. The message is
  the one that mechanism exists to give: treat `devcroft up` on an unread
  repository as running its code.
- The toolchain paths a Swift build needs are discovered from
  `swift -print-target-info`, which is **structured data and runs no
  project code**, and declared as `provider:swift` grants that
  `policy --render` shows. The artifact tier's cost becomes a visible
  difference in the compiled policy rather than a word in a document.
- `docs/decisions.md` §1 gains the entry recording that this provider is
  shipped *despite* failing the test, so the test keeps meaning
  something. `docs/known-gaps.md` publishes the execution gap.

## The objection, recorded

This contradicts CLAUDE.md's two-phase execution invariant
("Provider provisioning … runs host-side at `up`, *before* restrictions,
using the host's own network" is trusted because it runs "pinned tooling
from a lockfile, not project code"). For `swift`, provisioning runs
project code with the operator's full access, and the read-exfiltration
above shows that is not theoretical.

The alternative offered and declined was to reject SwiftPM as a provider
and serve Swift through the existing closure tier — nixpkgs ships the
Swift toolchain — which is how `docs/decisions.md` already answers Rust
(`rust-toolchain.toml` → a generated flox manifest). That option remains
available and is recorded here so the trade stays visible if this is
revisited.

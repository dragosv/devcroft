# Exclude the native macOS Swift provider from standard devcroft builds

## Why

Devcroft's contract requires every supported sandbox configuration to keep host shells and host base userland out of the sandbox. The current native macOS `swift` provider (`src/provider/swift.rs`) grants Xcode or Command Line Tools developer trees (which carry `usr/bin` host utilities made executable under Seatbelt's read-to-exec rule) and has no content-addressed shared store. Labeling that path "experimental" in the standard distribution would ship two incompatible promises under one binary, while qualifying native Apple/ncroft workflows is outside `devcroft 1.0` scope.

## What Changes

- Gate the executable `swift` provider implementation (`src/provider/swift.rs`) behind a non-default Cargo feature (`research-swift-provider`), disabled in default builds, CI release artifacts and `cargo install devcroft`. Keep the source in-tree for research and potential `ncroft` reuse without deleting it.
- Keep `env.provider = "swift"` recognized by manifest validation in standard builds solely to fail deterministically at layer `config` (exit code 2) with a clear message explaining that native macOS Swift is excluded from standard devcroft builds (naming closure-tier `nix`/`flox` for portable Swift packages, with zero silent fallback to another provider or `ncroft`).
- Ensure standard builds never autodetect `Package.swift` during `init` or provider discovery and never invoke `xcrun`, `xcode-select` or `swift`.
- Reconcile CLI help, `doctor`, test rows (`tests/common/mod.rs`), `README.md`, `docs/decisions.md` §1, `docs/known-gaps.md`, `samples/swift-spm-sample/README.md` and `openspec/config.yaml` with the standard distribution.

## Capabilities

### Modified Capabilities

- `config` recognizes `env.provider = "swift"` in standard builds to emit an explicit `config`-layer exclusion refusal rather than an opaque unknown-provider error or silent fallback.
- `env-provider` removes executable `swift` resolution and `Package.swift` autodetection from the standard build while keeping the non-default research feature isolated.
- `cli` updates `init`, `doctor` and help text so standard builds advertise only shipped providers that satisfy devcroft's non-host userland invariant.

## Impact

Affects `Cargo.toml`, `src/provider/{mod,validate,swift}.rs`, `src/lifecycle/up.rs`, `src/bin/devcroft.rs`, `tests/common/mod.rs`, `samples/swift-spm-sample/README.md` and public documentation. No other macOS provider (`nix`, `flox`, `devbox`, `devenv`, `mise`, `pixi`) is removed or restricted, and a future Swift profile that satisfies the non-host invariant remains open to separate qualification.

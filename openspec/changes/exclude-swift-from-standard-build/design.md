## Context

`src/provider/swift.rs` currently compiles unconditionally into `devcroft`, `SUPPORTED` in `src/provider/validate.rs` accepts `"swift"`, and `up` / `init` / `doctor` wire Swift toolchain discovery and Apple-framework scanning into the main binary. On macOS, `Toolchain::grants` grants `Xcode.app/Contents` or Command Line Tools paths (or `/usr/bin` when `xcrun` is absent), and `scope_exec_to_read_grants` (`src/policy/capability_set.rs`) allows `process-exec*` on every read grant — exposing host `usr/bin` utilities (`git`, `make`, `python3`, `clang`, shims) inside the sandbox and violating devcroft's non-host shell/base-userland invariant. Product direction for 1.0 (`docs/decisions.md` §1) excludes this native path from standard builds rather than shipping an "experimental" exception, while keeping `ncroft` out of 1.0 scope.

## Decisions

### S1. Non-default Cargo feature for research retention

Add a non-default Cargo feature `research-swift-provider` (not in `default = [...]`). `#[cfg(feature = "research-swift-provider")]` gates `pub mod swift`, `SwiftProvider`, the Apple-framework import scan in `up`, and Swift-specific shim/doctor helpers, so standard builds neither compile nor link executable Swift provider code. Research builds that explicitly pass `--features research-swift-provider` must print a prominent banner stating the build does not satisfy devcroft's non-host userland contract.

### S2. Explicit refusal at layer `config` with zero fallback

In standard builds (`not(feature = "research-swift-provider")`), `validate_provider("swift")` returns a dedicated `ProviderError` variant (mapped to layer `config`, exit code 2) stating that the native macOS `swift` provider is excluded from standard devcroft builds because its host developer-tree grants violate the non-host shell/base-userland contract, and naming `flox` / `nix` for portable Swift packages. Aliases (`swiftpm`, `spm`) remain `Unknown` as before. Neither `up` nor `init` falls back to another provider when `Package.swift` is present.

### S3. Release verification and documentation alignment

Add an integration test running against the default build (`cargo test`) that verifies:
1. `devcroft up` and `devcroft doctor` on a manifest with `env.provider = "swift"` exit with code 2 (layer `config`), naming the exclusion and executing neither `xcrun` nor `swift` (verified via a marker binary first on `PATH`).
2. `devcroft init` in a directory containing only `Package.swift` does not generate `provider = "swift"`.
3. `devcroft --help` and `devcroft init --help` list only providers shipped in the standard build.

Update `tests/common/mod.rs` so the `swift` test row exists only under `research-swift-provider`, and update `README.md`, `docs/decisions.md` §1, `docs/known-gaps.md`, `samples/swift-spm-sample/README.md`, `AGENTS.md` and `openspec/config.yaml` to record the 1.0 exclusion.

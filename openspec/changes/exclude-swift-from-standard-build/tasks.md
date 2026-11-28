## 1. Gate the Swift provider out of standard builds

- [ ] 1.1 Add non-default Cargo feature `research-swift-provider` in `Cargo.toml` and gate `src/provider/swift.rs` and its call sites in `src/provider/mod.rs`, `src/lifecycle/up.rs` and `src/bin/devcroft.rs` behind `#[cfg(feature = "research-swift-provider")]`. Regenerate `THIRD-PARTY-LICENSES.md` if `Cargo.lock` changes.
- [ ] 1.2 In `src/provider/validate.rs` (when `research-swift-provider` is disabled), reject `env.provider = "swift"` with an explicit `config`-layer error (exit code 2) explaining the non-host userland exclusion and naming closure-tier alternatives, while keeping `swiftpm`/`spm` as `Unknown` and ensuring zero fallback.
- [ ] 1.3 Remove `Package.swift` autodetection and `"swift"` from standard-build `init`, `doctor` and `--help` output; print a clear non-standard-contract notice when built with `--features research-swift-provider`.

## 2. Verify and align documentation

- [ ] 2.1 Gate the `swift` row in `tests/common/mod.rs` and existing Swift e2e tests behind `research-swift-provider`, and add a default-build integration test proving `provider = "swift"` exits 2 at layer `config` without invoking marker `xcrun`/`swift` binaries on `PATH` and `init` ignores `Package.swift`.
- [ ] 2.2 Update `README.md`, `CONTRIBUTING.md` (the `swift` test row), `docs/invariants.md` (the `native:<provider>` origin), `docs/base-runtime.md` (native mode), `docs/decisions.md` §1, `docs/known-gaps.md`, `samples/README.md`, `samples/swift-spm-sample/README.md`, `AGENTS.md` and `openspec/config.yaml` so standard 1.0 documentation never advertises native `swift` as an available provider.

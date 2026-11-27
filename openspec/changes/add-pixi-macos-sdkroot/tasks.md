## 1. Config, SDK discovery and policy flag

- [ ] 1.1 Parse `[env.pixi] macos_sdk` (`bool` or version string) in `src/config/`, refusing non-macOS use at layer `config` (exit code 2) when enabled.
- [ ] 1.2 Add `executable: bool` to read grants in `src/policy/` so `scope_exec_to_read_grants` on macOS omits `(allow process-exec* ...)` for data-only grants (`executable = false`), and display `(no-exec)` with origin `host:macos-sdk` in `policy --render`.
- [ ] 1.3 Implement canonical `MacOSX*.sdk` + `SDKSettings.json` discovery, version matching, `SDKROOT`/`CONDA_BUILD_SYSROOT` injection, `SDKSettings.json` staleness fingerprinting and `doctor` reporting in `src/provider/pixi.rs`.

## 2. Verify boundary and native build

- [ ] 2.1 Add macOS integration tests in `tests/pixi_provider_e2e.rs` verifying: a Conda C/Rust compile succeeds with `macos_sdk = true`, `policy --render` shows `host:macos-sdk` `(no-exec)` on `.../SDKs/MacOSX*.sdk` alone, `/Library/Developer/CommandLineTools/usr/bin/clang` and `/usr/bin/clang` remain denied (`EPERM`), executing a binary inside a synthetic `MacOSX.sdk` fixture fails `EPERM`, and `base-runtime` provenance records zero `host:` utilities.
- [ ] 2.2 Document `[env.pixi] macos_sdk` and the data-only `host:macos-sdk` exception in `docs/decisions.md` and `samples/pixi-sample/README.md`.

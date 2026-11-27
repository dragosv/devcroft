# Allow opt-in read-only macOS SDKROOT grants for Pixi native builds

## Why

Conda-forge compilers on macOS (`clang_osx-arm64`, `rust_osx-arm64`) cannot redistribute Apple's `MacOSX*.sdk` headers and `.tbd` link stubs, so native C/C++/Rust extensions in a `pixi` sandbox fail to compile unless an SDK sysroot is readable. Devcroft's invariant forbids host shells and host base userland (`/bin`, `/usr/bin`, `CommandLineTools/usr/bin`), not an explicitly declared, data-only platform SDK. Without a dedicated feature, users are tempted to grant `/Library/Developer/CommandLineTools` in `filesystem.read`, which exposes `CommandLineTools/usr/bin/{clang,git,make,python3}` as executable inside the sandbox.

## What Changes

- Add an opt-in `[env.pixi] macos_sdk = true` (or explicit version string such as `"15.2"`) in `devcroft.toml`, valid on macOS only and refused at layer `config` on non-macOS hosts or non-`pixi` providers.
- Resolve the canonical `MacOSX*.sdk` directory (`SDKSettings.json` verified) from Command Line Tools or Xcode without evaluating project files, refuse any path outside a canonical `.../SDKs/MacOSX*.sdk` bundle, and never grant parent developer directories containing `usr/bin`.
- Grant the resolved `.sdk` directory **read-only and non-executable** (carved out of macOS Seatbelt's `scope_exec_to_read_grants` `process-exec*` rules) under origin `host:macos-sdk`, inject `SDKROOT` and `CONDA_BUILD_SYSROOT` into the resolved environment when unset by the project, and record the SDK path, version and `SDKSettings.json` SHA-256 in `provider::pixi` fingerprinting and `doctor`.

## Capabilities

### Modified Capabilities

- `config` accepts `[env.pixi] macos_sdk` (`bool` or SDK version string) on `pixi` manifests and refuses it on non-macOS targets at `up`.
- `env-provider` discovers and fingerprints `MacOSX*.sdk` (`SDKSettings.json`), sets `SDKROOT` / `CONDA_BUILD_SYSROOT`, and surfaces missing SDKs in `doctor` and `up`.
- `policy` supports non-executable read-only grants (`executable = false`) so `host:macos-sdk` grants `file-read*` on `MacOSX*.sdk` without emitting `(allow process-exec* ...)` on macOS and without granting any `usr/bin` directory.

## Impact

Touches `src/config/`, `src/provider/pixi.rs`, `src/policy/{mod,capability_set}.rs`, `tests/pixi_provider_e2e.rs`, `samples/pixi-sample` and `docs/decisions.md`. Preserves the non-host shell and base-userland invariant completely while enabling native C/Rust compilation under `pixi` on macOS.

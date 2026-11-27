## ADDED Requirements

### Requirement: Canonical MacOSX.sdk resolution, environment export and staleness
When `[env.pixi] macos_sdk` is enabled on macOS, `provider::pixi` SHALL resolve a canonical `.../SDKs/MacOSX*.sdk` directory verified by `SDKSettings.json` without evaluating project files, export `SDKROOT` and `CONDA_BUILD_SYSROOT` to that path unless already set by the pixi environment, and include the canonical path and `SDKSettings.json` digest in the environment fingerprint and `doctor` output.

#### Scenario: Native C or Rust build in a Pixi macOS sandbox links against SDKROOT
- **WHEN** a `pixi` project on macOS declares `clang` or `rust` from `conda-forge` and sets `[env.pixi] macos_sdk = true`
- **THEN** `SDKROOT` and `CONDA_BUILD_SYSROOT` point to the canonical `MacOSX*.sdk` directory and compiling a binary inside the sandbox succeeds

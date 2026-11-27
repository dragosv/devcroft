## Context

On macOS, `pixi` installs compilers and tools from `conda-forge` (`add-pixi-provider`), which link against Apple system frameworks and headers via `SDKROOT` / `CONDA_BUILD_SYSROOT`. Two codebase facts govern how an Apple SDK may be exposed:
1. `/Library/Developer/CommandLineTools` and `Xcode.app/Contents/Developer` contain both `SDKs/MacOSX*.sdk` (headers, modulemaps, `.tbd` text stubs) and `usr/bin` (`clang`, `git`, `make`, `python3`, `ld`). Granting the parent developer directory violates the non-host userland invariant.
2. On macOS, `scope_exec_to_read_grants` (`src/policy/capability_set.rs:446-468`) currently emits `(allow process-exec* (subpath ...))` for every `AccessMode::Read` grant. A data-only SDK needs `file-read*` only, never `process-exec*`.

## Decisions

### P1. Explicit opt-in `[env.pixi] macos_sdk`

Extend `EnvPixi` in `src/config/` with `macos_sdk: Option<MacosSdkSelector>` (`false` / omitted by default; `true` selects the active default `MacOSX.sdk`; a string `"15"` or `"15.2"` selects the matching `MacOSX<ver>.sdk`). On Linux, `macos_sdk = true` or `"..."` fails at layer `config` (exit code 2) before provider resolution; `macos_sdk = false` is a no-op everywhere so a cross-platform `devcroft.toml` only opts in when macOS native linking is specifically required (or if a target-scoped key is later added, without silent degradation).

### P2. Strict `SDKs/MacOSX*.sdk` discovery and fingerprinting

Discover candidate SDK roots from `/Library/Developer/CommandLineTools/SDKs` and `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs` (or `xcrun --sdk macosx --show-sdk-path` under a clean environment when needed), canonicalize the chosen directory, and verify:
- Its canonical path ends inside `.../SDKs/MacOSX*.sdk` and contains a regular `SDKSettings.json` file whose parsed `Version` matches any requested version prefix.
- No parent directory (`CommandLineTools`, `Developer`, `usr/bin`) is added to grants.
- Export `SDKROOT` and `CONDA_BUILD_SYSROOT` to the canonical `.sdk` path unless the project's pixi activation already set them. Include the canonical `.sdk` path and `sha256(SDKSettings.json)` in `pixi::fingerprint()` so SDK upgrades mark the sandbox stale in `status`.

### P3. Data-only read grant (`executable: false`) with origin `host:macos-sdk`

Add an `executable: bool` flag (default `true` for existing provider/runtime grants; `false` for data-only grants) on resolved read grants:
- On macOS, `scope_exec_to_read_grants` skips `executable == false` grants when emitting `(allow process-exec* ...)`, so nothing inside `MacOSX*.sdk` can be `execve`'d.
- Render the grant in `devcroft policy --render` as `read (no-exec)` with origin `host:macos-sdk`, and ensure `base-runtime` provenance still reports zero `host:` entries for shell and base userland.

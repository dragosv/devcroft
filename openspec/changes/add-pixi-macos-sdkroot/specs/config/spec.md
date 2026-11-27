## ADDED Requirements

### Requirement: Opt-in Pixi macOS SDK selector
`devcroft.toml` SHALL accept `macos_sdk` (`true`, `false`, or an SDK version string) under `[env.pixi]`. Enabling `macos_sdk` on a non-macOS host, or when no matching `MacOSX*.sdk` with `SDKSettings.json` exists on the macOS host, SHALL fail closed before starting a keeper (layer `config` on non-macOS; layer `provider` when the requested macOS SDK is missing).

#### Scenario: Enabling macos_sdk on Linux fails closed
- **WHEN** `devcroft.toml` sets `[env.pixi] macos_sdk = true` on a Linux host
- **THEN** `up` fails at layer `config` (exit code 2) naming `macos_sdk` and the host platform

#### Scenario: Missing requested SDK version fails with available versions
- **WHEN** `devcroft.toml` sets `[env.pixi] macos_sdk = "99.0"` on macOS
- **THEN** `up` fails at layer `provider` listing the discovered `MacOSX*.sdk` versions

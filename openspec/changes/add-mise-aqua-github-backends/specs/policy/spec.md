## ADDED Requirements

### Requirement: Expanded mise backends preserve narrow provider grants
Tools materialized via `aqua:`, `github:` or `conda:` under `provider::mise` SHALL be granted read-only under `~/.local/share/devcroft-artifacts/mise/<id>` with origin `provider:mise`, alongside only the minimal host loader/libc library paths inspected from their binary headers. Neither `/usr/bin` nor any host shell directory SHALL be granted.

#### Scenario: Rendered policy shows narrow provider:mise grants and no host toolchain
- **WHEN** a `mise` sandbox using `aqua:` or `conda:` tools is up and `devcroft policy --render` runs
- **THEN** the artifact object appears read-only with origin `provider:mise` and neither `/usr/bin` nor `/bin` appears as a host grant

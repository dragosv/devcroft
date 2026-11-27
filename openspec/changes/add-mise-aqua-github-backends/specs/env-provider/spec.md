## ADDED Requirements

### Requirement: Locked aqua, GitHub and public-host Conda materialization
`provider::mise` SHALL admit `mise.lock` entries whose `backend` uses `http:`, `aqua:`, `github:` or `conda:` (including bare registry tool names in `mise.toml` that resolve to one of those backends in `mise.lock`) when every artifact for the current platform has an `https://` URL without userinfo or fragments and a 64-hex `sha256:` checksum. For `conda:` entries, every artifact URL host SHALL be restricted to `conda.anaconda.org` or `repo.prefix.dev`. `mise install --locked` SHALL run under an isolated environment without executing downloaded payloads or requiring external host signature-verifier binaries, and SHALL leave `mise.lock` (and any consumed lock sidecar) byte-identical.

#### Scenario: Portable aqua or github tool restores across platforms from one mise.toml
- **WHEN** a project commits `mise.toml` with an `aqua:` or `github:` tool (or bare shorthand resolving to `aqua:`) and a `mise.lock` containing `macos-arm64` and `linux-arm64` URLs and `sha256:` checksums
- **THEN** `devcroft up` materializes the read-only object without modifying `mise.toml` or `mise.lock`, and the tool runs inside the sandbox

#### Scenario: Private Conda channel is refused before subprocess execution
- **WHEN** `mise.lock` contains a `conda:` artifact URL outside `conda.anaconda.org` and `repo.prefix.dev`
- **THEN** resolution fails naming the host before `mise` is invoked

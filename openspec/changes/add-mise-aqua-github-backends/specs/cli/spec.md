## ADDED Requirements

### Requirement: Mise doctor reports the non-host shell contract
`devcroft doctor` on a `mise` project SHALL report the qualified `mise` version, the count and backends of locked tools for the current target, and the non-host shell resolution (an environment shell if locked, otherwise devcroft's embedded runtime shell), without canonicalizing or reporting the host's `/bin/sh`.

#### Scenario: Doctor on a shell-less mise project names the runtime shell
- **WHEN** `devcroft doctor` runs on a locked `mise` project that declares no POSIX shell
- **THEN** the output names devcroft's runtime shell rather than `/bin/sh` or `/bin/zsh` on the host

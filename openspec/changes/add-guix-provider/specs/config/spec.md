## ADDED Requirements

### Requirement: guix provider value
The system SHALL accept `env.provider = "guix"` as a closure-tier provider
once this change's qualification admits it. Until then, `guix` SHALL fail at
layer `config`, exit code 2, as not yet supported.

#### Scenario: Admitted
- **WHEN** the manifest declares `provider = "guix"` on Linux
- **THEN** validation succeeds and `status` reports tier `closure`

### Requirement: Optional guix selection table
The system SHALL accept an optional `[env.guix]` table holding only `manifest`
and `channels`, paths relative to the project root, defaulting to
`manifest.scm` and `channels.scm`. Unknown keys, paths that leave the
canonical project root, and the table under another provider SHALL fail at
layer `config`.

#### Scenario: Escaping path
- **WHEN** `[env.guix] channels = "../channels.scm"`
- **THEN** validation fails at layer `config` naming the key

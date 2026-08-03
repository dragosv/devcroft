# policy Specification

## Purpose

Compile the manifest deterministically into a backend-native sandbox profile
(nono profile JSON in MVP), make the compiled artifact inspectable, and
explain individual allow/deny decisions.

## ADDED Requirements

### Requirement: Deterministic compilation
The system SHALL compile the manifest plus provider-derived grants into a
single backend profile, byte-identical for identical inputs, written to
`<state>/<name>/profile.json` at `up`.

#### Scenario: Reproducible profile
- **WHEN** `up` runs twice with unchanged manifest and provider state
- **THEN** the compiled profile files are byte-identical

### Requirement: Baseline denials
The system SHALL always deny read and write access to devcroft's own data
dir (client keys) and, unless explicitly allowed, to known credential
directories, regardless of manifest contents.

#### Scenario: Client key is unreachable
- **WHEN** any session inside the sandbox attempts to read
  `~/.local/share/devcroft/id_ed25519`
- **THEN** the kernel denies the access

### Requirement: Inspectable policy
The system SHALL provide `devcroft policy --render [--backend nono]`
printing the compiled profile, and SHALL annotate each rule with its origin
(`manifest:<key>` | `provider:<name>` | `baseline`).

#### Scenario: Tracing a rule to its source
- **WHEN** the user runs `policy --render`
- **THEN** the read grant for `/nix/store` is annotated `provider:flox`

### Requirement: Explainable decisions
The system SHALL provide `devcroft why --path <p> --op <read|write>` (and
`--host <domain>` for network) answering ALLOWED or DENIED with the
responsible rule, delegating to the backend's own explainer where one
exists.

#### Scenario: Explaining a denial
- **WHEN** the user runs `why --path ~/.aws/credentials --op read`
- **THEN** the output is `DENIED` with rule origin `baseline` and the
  matching pattern

### Requirement: Degraded capability surfacing
The system SHALL compare requested policy aspects against what the backend
can enforce on the current host, and report unenforceable aspects once at
`up` with severity `warning`, never silently dropping them.

#### Scenario: Domain allowlist on macOS
- **WHEN** the manifest declares `network.allow` domains
- **AND** the host backend cannot enforce domain filtering
- **THEN** `up` succeeds and prints exactly one warning naming the aspect,
  the reason, and the effective fallback behavior

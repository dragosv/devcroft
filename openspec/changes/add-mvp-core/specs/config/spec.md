# config Specification

## Purpose

Define, parse, and validate the declarative manifest (`devcroft.toml`) that
is the single source of truth for a sandbox: its identity, environment
provider, filesystem policy, network policy, and hooks.

## ADDED Requirements

### Requirement: Manifest discovery
The system SHALL locate the manifest by searching for `devcroft.toml` in the
current directory and its ancestors, stopping at the filesystem root or a
`.git` boundary, whichever comes first.

#### Scenario: Run from a subdirectory
- **WHEN** the user runs any subcommand from `<root>/src/deep/`
- **AND** `<root>/devcroft.toml` exists
- **THEN** that manifest is used and `<root>` is the sandbox project root

#### Scenario: No manifest found
- **WHEN** no `devcroft.toml` exists in the ancestor chain
- **AND** the subcommand requires one (`up`, `shell`, `exec`, `ssh`)
- **THEN** the command fails with exit code 2 and a message suggesting
  `devcroft init`

### Requirement: Manifest schema
The system SHALL accept a TOML manifest with sections `[sandbox]`, `[env]`,
`[filesystem]`, `[network]`, `[ssh]`, `[hooks]`, where only `[sandbox].name`
is mandatory and every other field has a documented default.

#### Scenario: Minimal manifest
- **WHEN** the manifest contains only `[sandbox] name = "myproj"`
- **THEN** validation succeeds with defaults: provider `flox`,
  filesystem allow `["."]`, network default `deny`, no hooks
- **AND** if no flox environment exists at `up`, the failure is at the
  provider layer with a hint to run `flox init` (see env-provider spec)

#### Scenario: Unknown key
- **WHEN** the manifest contains a key not in the schema
- **THEN** validation fails with the exact key path and closest valid
  alternative (typo suggestion), and exit code 2

### Requirement: Static environment variables
The system SHALL accept `[env] vars` as a table of static environment
variables injected into every session, applied AFTER provider resolution
so they can override provider-set values. Values are literal strings; no
host-environment interpolation is performed (that would leak
non-reproducible host state).

#### Scenario: Var overrides provider value
- **WHEN** the provider sets `FOO=a` and the manifest sets
  `vars = { FOO = "b" }`
- **THEN** sessions see `FOO=b`

#### Scenario: No host interpolation
- **WHEN** the manifest sets `vars = { TOKEN = "$HOST_TOKEN" }`
- **THEN** sessions see the literal string `$HOST_TOKEN`, and validation
  prints a one-time warning that interpolation is not supported

### Requirement: Name constraints
The system SHALL restrict `[sandbox].name` to `[a-z0-9][a-z0-9-]{0,31}`,
because the name becomes a hostname label (`<name>.devcroft`) and a state
directory component.

#### Scenario: Invalid name
- **WHEN** `name = "My Project"`
- **THEN** validation fails naming the constraint and showing a suggested
  slug (`my-project`)

### Requirement: Filesystem policy validation
The system SHALL validate that every path in `filesystem.allow`,
`filesystem.read`, and `filesystem.deny` is either relative to the project
root or an absolute/tilde path, and SHALL reject rules that are provably
useless (deny of a path never granted).

#### Scenario: Deny wins over allow
- **WHEN** `allow = ["~"]` and `deny = ["~/.ssh"]`
- **THEN** validation succeeds and the compiled policy grants `~` minus
  `~/.ssh`

#### Scenario: Sensitive-path warning
- **WHEN** `allow` includes a known credential directory (`~/.ssh`, `~/.aws`,
  `~/.config/gcloud`, `~/.kube`)
- **THEN** validation succeeds but a warning is printed at `up`, once

### Requirement: Network policy model
The system SHALL model network policy as `default = "deny" | "allow"` plus
an `allow` list of domain names, and SHALL treat domain filtering as a
capability that may be unenforceable on a given host (see policy spec).

#### Scenario: Domains with deny default
- **WHEN** `default = "deny"` and `allow = ["github.com", "crates.io"]`
- **THEN** validation succeeds and the compiled policy requests a
  proxy-backed allowlist from the backend

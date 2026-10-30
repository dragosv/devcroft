# config Delta Specification (add-agent-workload)

## ADDED Requirements

### Requirement: User configuration file
The system SHALL read an optional user configuration file at
`$XDG_CONFIG_HOME/devcroft/config.toml` (`~/.config/devcroft/config.toml`
when unset, on every platform), holding only the tooling layer and
credential requests, validated as strictly as `devcroft.toml`, failing
at layer `config` with exit code 2. An absent file SHALL change nothing.

#### Scenario: Unknown key in the user configuration
- **WHEN** the user configuration contains a key the schema does not
  define
- **THEN** validation fails at layer `config` with exit code 2, naming
  the file and the full key path

#### Scenario: Absent file is inert
- **WHEN** no user configuration file exists and `--tools` is not given
- **THEN** the compiled policy and captured environment are identical to
  the same project before this capability existed

### Requirement: Tooling layer declaration
The system SHALL accept a `[tools]` section in the user configuration
file naming the layer's provider and the directory holding its
environment, and SHALL accept the same as `up --tools <path>`. The
system SHALL NOT accept a `[tools]` section in `devcroft.toml`.

#### Scenario: Layer declared in the user configuration
- **WHEN** the user configuration has `[tools]` with a provider and a
  path
- **THEN** it parses, and the layer is resolved at the next `up`

#### Scenario: Layer declared in the project is refused
- **WHEN** `devcroft.toml` contains `[tools]`
- **THEN** validation fails at layer `config` with exit code 2, naming
  the key

### Requirement: Credential request declaration
The system SHALL accept an explicit credential request in the user
configuration file, distinguishing the environment-variable shape from
the single-file shape, and SHALL validate at parse time that a
file-shaped request names a file rather than a directory. A credential
request SHALL NOT accept a glob or a path pattern matching more than one
file, and SHALL NOT be accepted in `devcroft.toml`. The project-level
`[env] forward` key is unchanged by this requirement.

#### Scenario: Directory rejected at parse time
- **WHEN** a file-shaped credential request names a directory
- **THEN** validation fails at layer `config` with exit code 2, before
  any sandbox is created

#### Scenario: Pattern rejected
- **WHEN** a credential request uses a glob or wildcard
- **THEN** validation fails, since a credential is granted per named file

### Requirement: Manifest-declared name remains optional to override
The system SHALL continue to treat `sandbox.name` as the declared
identity, while allowing an invocation-level override. A manifest SHALL
remain valid unchanged, and the presence of an override SHALL NOT require
any manifest edit.

#### Scenario: Manifest unchanged by an override
- **WHEN** a sandbox is brought up with an overridden name
- **THEN** `devcroft.toml` on disk is not modified, and remains valid

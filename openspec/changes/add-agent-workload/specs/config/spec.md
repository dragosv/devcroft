# config Delta Specification (add-agent-workload)

## ADDED Requirements

### Requirement: Tooling layer declaration
The system SHALL accept a `[tools]` section in `devcroft.toml` declaring
a second declarative environment, validated with the same strictness as
existing sections: unknown keys are rejected with the full key path, and
a malformed section fails at layer `config` with exit code 2. Omitting
the section SHALL be equivalent to declaring no tooling layer, and SHALL
change nothing about a sandbox's behavior.

#### Scenario: Unknown key in the tooling section
- **WHEN** `[tools]` contains a key the schema does not define
- **THEN** validation fails at layer `config` with exit code 2, naming
  the full key path, consistent with every other section

#### Scenario: Omitted section is inert
- **WHEN** a manifest declares no `[tools]` section
- **THEN** the compiled policy and captured environment are identical to
  the same manifest before this capability existed

### Requirement: Credential request declaration
The system SHALL accept an explicit credential request in
`devcroft.toml`, distinguishing the environment-variable shape from the
single-file shape, and SHALL validate at parse time that a file-shaped
request names a file rather than a directory. A credential request SHALL
NOT accept a glob or a path pattern matching more than one file.

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

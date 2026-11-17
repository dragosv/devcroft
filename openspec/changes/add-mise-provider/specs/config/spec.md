## ADDED Requirements

### Requirement: mise provider selection
The system SHALL accept `mise` as an `env.provider` value with no aliases. It
SHALL preserve the default provider and all existing provider values, and SHALL
continue refusing `host` and `none`. Acceptance of the name SHALL NOT imply
acceptance of every mise backend or configuration form.

#### Scenario: Explicit mise project
- **WHEN** a valid devcroft manifest declares `env.provider = "mise"`
- **THEN** configuration resolves to provider `mise`

#### Scenario: Existing defaults preserved
- **WHEN** a manifest omits the provider or names an existing supported provider
- **THEN** provider selection and compiled policy remain unchanged

#### Scenario: Passthrough remains refused
- **WHEN** a manifest selects `host` or `none`
- **THEN** validation fails at layer `config`, exit code 2

### Requirement: Bounded mise project configuration
The system SHALL select exactly one root manifest, `mise.toml` or `.mise.toml`,
and SHALL refuse by path every other project-level configuration file or
version file the qualified mise version would load, before invoking mise. A
missing manifest SHALL be a provider error, not an unknown-provider error.

#### Scenario: Two competing manifests
- **WHEN** both root manifest names exist
- **THEN** `up` fails at layer `provider`, exit code 3, naming both paths
- **AND** no installation or project code runs

#### Scenario: A second configuration location
- **WHEN** the project also contains `.config/mise.toml`, a `conf.d` fragment, a local override or an environment profile
- **THEN** `up` refuses it by path instead of resolving without it

### Requirement: Every mise table is consumed, refused or stated
The system SHALL classify each top-level table of the selected manifest. It
SHALL consume `[tools]` and `[env]`, SHALL read only the allowed `[settings]`
keys and refuse any other settings key by dotted name whatever its value, and
SHALL refuse `[hooks]`, `[watch_files]`, `[plugins]`, tool aliases and unknown
tables by name. It SHALL NOT carry `[tasks]`, and SHALL say so in `doctor`
rather than refuse them.

#### Scenario: A setting that weakens verification
- **WHEN** the manifest sets a `[settings]` key outside the allowed set to `false`
- **THEN** `up` fails at layer `provider`, exit code 3, naming the dotted key
- **AND** the value being `false` does not make it a non-declaration

#### Scenario: Shell-integration hook
- **WHEN** the manifest declares a `[hooks]` entry
- **THEN** `up` refuses it by key without running it or moving it to another phase

#### Scenario: Tasks are present
- **WHEN** the manifest declares `[tasks]`
- **THEN** `up` succeeds and `doctor` states that `mise run` is not available inside the sandbox

### Requirement: mise daemons are refused until supported
While no change supplies mise service support, the system SHALL refuse a
`[daemons]` table in the selected manifest by name at layer `provider`, exit
code 3, and SHALL NOT report it as an empty service set. `up --skip-hooks`
SHALL NOT fail on it.

#### Scenario: Daemons declared
- **WHEN** a mise project declares `[daemons]` and starts without `--skip-hooks`
- **THEN** `up` fails naming `[daemons]` and stating that mise services are not yet supported

#### Scenario: Escape hatch
- **WHEN** the same project starts with `--skip-hooks`
- **THEN** `up` proceeds without starting or refusing them

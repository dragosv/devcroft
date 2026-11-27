## ADDED Requirements

### Requirement: File-only selection on the command line
`devcroft up` and `devcroft agent run` SHALL accept `--file-only`, listed in
help and in `USAGE`. `--file-only` SHALL be refused at layer `config` together
with flags that need a provider or a manifest, naming the conflicting flag.
`status` SHALL show each sandbox's profile (`managed` or `file-only`).

#### Scenario: Help lists the flag
- **WHEN** `devcroft --help` runs
- **THEN** `--file-only` is listed for `up` and `agent run`

#### Scenario: Status of both profiles
- **WHEN** a managed and a file-only sandbox are up and `devcroft status` runs
- **THEN** each line names its profile

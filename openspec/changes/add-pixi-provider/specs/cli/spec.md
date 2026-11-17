## ADDED Requirements

### Requirement: Pixi discovery and diagnostics are read-only
`init` SHALL discover Pixi by reading eligible TOML files and SHALL follow the
existing explicit provider-choice behavior for ambiguity. `doctor` SHALL check
the pixi binary and version against the qualified range, the selected manifest
and environment, the lock and platform, the packages that would be refused and
the artifact store's size, and SHALL state that Pixi tasks cannot be run by
name inside the sandbox. Neither SHALL install, activate or run tasks. A
missing Pixi prerequisite SHALL be distinguished from an unknown provider.

#### Scenario: Discovery in a Python workspace
- **WHEN** `init` finds `[tool.pixi]` in `pyproject.toml`
- **THEN** it recognizes Pixi without invoking Python, pixi or a build backend

#### Scenario: Missing executable
- **WHEN** Pixi is selected and no qualified executable is available
- **THEN** `doctor` names the prerequisite and `up` fails at layer `provider`, exit code 3

### Requirement: Pixi reports guarantee, selection and support scope
`up` and `status` SHALL identify the selected Pixi environment and the
artifact tier with its host-runtime dependence. The staleness line SHALL name
the Pixi manifest. Errors SHALL keep the existing layers and exit codes. A
pixi version below the qualified minimum SHALL be refused; one newer than the
last qualified SHALL draw exactly one warning naming that version.
Documentation SHALL list the supported and refused environment shapes and
SHALL NOT present native support as VM or fleet support.

#### Scenario: Named environment starts
- **WHEN** the `test` environment is selected and starts
- **THEN** `status` shows `pixi`, `test` and the artifact-tier line
- **AND** nothing implies closure-tier or cross-platform equivalence

#### Scenario: Unsupported editable project
- **WHEN** a workspace requests an editable package
- **THEN** the error names the dependency and that its build backend would have to run
- **AND** the package is neither built on the host nor silently omitted

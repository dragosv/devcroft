## ADDED Requirements

### Requirement: mise discovery and provider diagnostics
`init` SHALL recognize a root mise manifest by reading files only, using the
existing explicit selection when several providers are present. `doctor` SHALL
scope mise checks to projects that select mise and SHALL report the mise
binary and version against the qualified range, lock coverage for this
platform, tools whose backend would be refused, the shell that would be
resolved, the artifact store's size, and that `[tasks]` cannot be run by name
inside the sandbox. Neither command SHALL install tools or execute project
configuration.

#### Scenario: Initialization cannot run a project template
- **WHEN** `init` examines a mise manifest containing templates or hooks
- **THEN** it discovers the provider without invoking mise

#### Scenario: Doctor on an unqualified backend
- **WHEN** `doctor` runs in a project whose lock names a backend outside the admitted list
- **THEN** it names the tool and backend and installs nothing

### Requirement: mise support claims and errors are scoped
`up` and `status` SHALL state the artifact guarantee and the host-runtime
dependence. Errors SHALL use the existing layers and codes: `config`/2,
`provider`/3, `backend`/4, `keeper`/5. The staleness line SHALL name the mise
manifest. A mise version below the qualified minimum SHALL be refused; one
newer than the last qualified SHALL draw exactly one warning naming that
version. Native success SHALL NOT be presented as VM, companion or fleet
qualification.

#### Scenario: Successful start reports the guarantee
- **WHEN** a qualified mise sandbox starts
- **THEN** `up` prints the artifact-tier line and `status` retains it
- **AND** `policy --render` shows the provider grants and what resolution runs host-side

#### Scenario: Newer mise than qualified
- **WHEN** the installed mise is newer than the last qualified version
- **THEN** `up` prints one warning naming the qualified version and proceeds

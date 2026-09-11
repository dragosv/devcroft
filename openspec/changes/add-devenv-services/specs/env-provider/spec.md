# env-provider Delta Specification (add-devenv-services)

## ADDED Requirements

### Requirement: devenv resolution captures service declarations
Resolution of a devenv environment SHALL capture the project's declared
processes alongside the environment and the activation script, in the
same host-side phase and under the same rule: as data, through an entry
point that executes no project code.

The captured declarations SHALL be reported as supported-with-zero for a
project that declares none, and never as "this provider has no service
concept" — the distinction is what lets a manifest asking for services
fail loudly instead of silently starting nothing.

Resolution SHALL NOT start, attach to, or query a running supervisor to
learn what is declared. devenv runs its own supervisor; devcroft never
touches that instance, and reading declarations is not an exception.

#### Scenario: Declarations captured with the environment
- **WHEN** `up` resolves a devenv project that declares processes
- **THEN** the declarations are captured in the same resolution that
  captures the environment and the activation script
- **AND** sessions started later inherit the resolved environment without
  the declarations being read again

#### Scenario: Reading declarations does not run enterShell
- **WHEN** a devenv project defines `enterShell` with a side effect
  outside the project root and declares processes
- **THEN** resolution reads the declarations and the side effect has not
  occurred when `up` returns

#### Scenario: Zero declared is not "unsupported"
- **WHEN** a devenv project declares no processes
- **THEN** resolution reports services as supported with an empty set

### Requirement: devbox service support is refused for a measured reason
The system SHALL report devbox's services as unsupported, and the reason
SHALL be the measured one rather than a deferral. A devbox project whose
devcroft manifest declares services SHALL fail distinguishably from a
provider that supports services and has none declared.

Measured against devbox 0.17.5, three independent reasons, any one of
which is sufficient:

1. `devbox.json` carries no service or process schema — its own
   configuration keys are `packages`, `env`, `env_from`, `include`,
   `init_hook`, `scripts` and the `environment*` family.
2. Plugin-supplied services arrive as generated per-plugin
   `process-compose.yaml` files under devbox's own cache directory,
   which is the artifact shape `add-flox-services` decision 1 refused.
3. `devbox services ls` executes `shell.init_hook` — measured against a
   sentinel, with `devbox install` and `devbox shellenv --pure` as
   zero-execution controls. Enumeration therefore runs project code
   host-side, which the services capability forbids.

The message SHALL name devbox's own unmet requirement and SHALL NOT
suggest switching providers. Reason 3 is the one to re-measure first if
devbox later adds a declaration schema, since a schema alone would not
make enumeration hook-free.

#### Scenario: A devbox project asking for services fails distinguishably
- **WHEN** a devcroft manifest declares services and names
  `provider = "devbox"`
- **THEN** `up` fails at layer `provider`, naming devbox's own reason
- **AND** the failure is distinguishable from a provider that supports
  services and has none declared

#### Scenario: The refusal does not recommend another provider
- **WHEN** devbox's services are reported as unsupported
- **THEN** the message states what devbox does not provide, and does not
  advise adopting a different environment provider

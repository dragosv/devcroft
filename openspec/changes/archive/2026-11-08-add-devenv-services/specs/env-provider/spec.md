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

The captured set SHALL be every process the provider's evaluation
produces, not only those the project wrote by hand. Measured on devenv
2.2.2: enabling one of devenv's own service integrations contributes a
process, whose command is a store path. Filtering those out would
require distinguishing them — the provider does not mark them — and
would drop exactly the services a user enabling an integration expects
to be supervised.

Because that widens what "declared" means, the system SHALL make the
origin discoverable: a user who never wrote a process by that name and
sees it supervised SHALL be able to find out why from devcroft's own
documentation and sample, not from its source.

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

#### Scenario: An integration's process is supervised like any other
- **WHEN** a devenv project enables one of the provider's own service
  integrations and declares no process of that name itself
- **THEN** that process is captured, supervised, and enumerable exactly
  as a handwritten one is

#### Scenario: Zero declared is not "unsupported"
- **WHEN** a devenv project declares no processes
- **THEN** resolution reports services as supported with an empty set

### Requirement: devbox service support is refused for a measured reason
The system SHALL report devbox's services as unsupported, and the reason
SHALL be the measured one rather than a deferral. A devbox project whose
devcroft manifest declares services SHALL fail distinguishably from a
provider that supports services and has none declared.

Measured against devbox 0.17.5. The reason is that **the declarations
are not the project's**: `devbox.json` carries no service or process
schema — its own configuration keys are `packages`, `env`, `env_from`,
`include`, `init_hook`, `scripts` and the `environment*` family — and
plugin-supplied services arrive as generated per-plugin
`process-compose.yaml` files under devbox's cache directory, authored by
the plugin rather than by the project. Someone writing a package name
into `devbox.json` has not declared a service.

`devbox services ls` does execute `shell.init_hook` (sentinel-measured,
with `devbox install` and `devbox shellenv --pure` as zero-execution
controls). That rules out one route rather than every route: `devbox
install` alone writes those files with zero hook executions, and reading
a file executes nothing. This spec previously called that measurement
decisive, which overstated it — recorded here because a rejection whose
stated reason is misranked is worse than one that is honest about being
a judgement.

The message SHALL name devbox's own unmet requirement and SHALL NOT
suggest switching providers. `docs/decisions.md` carries what
reconsidering would cost, measured.

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

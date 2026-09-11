# env-provider Delta Specification (add-devbox-services)

## ADDED Requirements

### Requirement: devbox resolution captures plugin service declarations
Resolution of a devbox environment SHALL capture the service
declarations its plugins supply, host-side, as data, through a route that
executes no project code.

`devbox services ls` SHALL NOT be that route. Measured against devbox
0.17.5: it executes `shell.init_hook`, where `devbox install` and
`devbox shellenv --pure` execute nothing. The declarations are already
on disk after installation, and reading a file executes nothing.

A devbox project whose plugins declare no services SHALL report services
as supported with an empty set, not as unsupported — the distinction that
lets a manifest asking for services fail loudly rather than start
nothing.

#### Scenario: Declarations captured at resolution
- **WHEN** `up` resolves a devbox project whose plugins ship service
  declarations
- **THEN** they are captured in the same host-side phase as the
  environment, and sessions started later inherit the resolved
  environment without them being read again

#### Scenario: The hook-running route is not used
- **WHEN** the system needs to know which services a devbox project has
- **THEN** it does not invoke a command that executes the project's
  activation hook

#### Scenario: No plugin services is not "unsupported"
- **WHEN** a devbox project's packages ship no service declarations
- **THEN** resolution reports services as supported with an empty set

### Requirement: A plugin declaration the system cannot carry is refused
Where a plugin's declaration uses a field the system cannot represent and
pass to its supervisor, `up` SHALL fail at layer `provider` naming the
plugin, the process and the field. The system SHALL NOT start a partially
translated service.

Measured across the `postgresql`, `redis`, `nginx` and `mysql` plugins,
nothing currently needs refusing: they use only `command`, `is_daemon`,
`shutdown.command`, `availability.restart`, `availability.max_restarts`
and `readiness_probe.exec`, all of which translate. The requirement
therefore guards against plugins not yet surveyed rather than against
present behaviour, and that is why it is stated rather than assumed
unnecessary.

#### Scenario: An unrepresentable field stops `up`
- **WHEN** a plugin declares a field the system cannot translate
- **THEN** `up` fails at layer `provider`, naming the plugin, the process
  and the field, and no service from that project starts

#### Scenario: The surveyed vocabulary translates whole
- **WHEN** a plugin declares a command, daemon flag, shutdown command,
  restart policy with a maximum, and an exec readiness probe
- **THEN** every one of them reaches the supervisor

## REMOVED Requirements

### Requirement: devbox service support is refused for a measured reason
**Reason**: The contract argument it recorded — that plugin-authored
declarations are not the project's — was decided against by the project
owner rather than refuted. The mechanical costs it listed have both
moved: readiness probes were built (`add-service-readiness`), and the
hook-executing route it called decisive was only ever one of two, the
other being to read the files devbox has already written.

**Migration**: devbox projects that previously failed when services were
declared now run them. `docs/decisions.md` keeps the argument that was
decided against, so the trade stays legible.

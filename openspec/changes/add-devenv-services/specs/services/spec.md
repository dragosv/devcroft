# services Delta Specification (add-devenv-services)

## ADDED Requirements

### Requirement: Service declarations are read as data, never by running project code
The system SHALL obtain a provider's service declarations through an
entry point that executes no project-defined code, and SHALL read them
host-side at `up` as data. Starting, executing, or entering anything to
discover *what* is declared is forbidden: enumeration is not activation.

This is the two-phase rule applied to the one place it had not yet been
checked. A provider's service command is project code and runs inside
the sandbox; discovering that such a command exists must not itself run
project code on the host.

Where a provider has service declarations but no entry point that
returns them without executing project code, the system SHALL report
that provider's services as unsupported, naming the reason. It SHALL NOT
read them through an entry point that runs project code, and SHALL NOT
support them partially.

#### Scenario: Reading declarations runs no project code
- **WHEN** a project defines both an activation hook with an observable
  side effect and one or more services, and `up` resolves it
- **THEN** the services are known to devcroft
- **AND** the hook's side effect has not occurred when `up` returns

#### Scenario: A provider with no hook-free way to enumerate is refused
- **WHEN** a provider can only list its services through a command that
  executes project code
- **THEN** that provider's services are reported as unsupported with
  that reason
- **AND** the system does not fall back to the code-executing command

### Requirement: devenv declares services in its own manifest
The system SHALL treat a devenv project's `processes` as declared
services. The declarations SHALL be read from devenv's own evaluated
manifest as structured data.

This meets the same bar flox met and for the same reason: the
declarations come from a documented option in the provider's own
manifest, not from a generated artifact. `add-flox-services` decision 1
refused flox's generated `service-config.yaml` on that ground, and that
refusal stands — a generated file is not a contract, whoever generates
it.

Measured against devenv 2.2.2: `devenv eval processes` returns every
declared process as normalized data and executes `enterShell` zero
times. The requirement is stated as a property of the result rather than
as a command name, so it survives devenv changing its CLI.

#### Scenario: Declared processes become supervised services
- **WHEN** a devenv project declares a process and the sandbox comes up
- **THEN** that process runs as a service inside the sandbox, supervised
  by the keeper, enumerable through the sandbox's own introspection, and
  reaped at teardown

#### Scenario: A devenv project declaring nothing is not "unsupported"
- **WHEN** a devenv project declares no processes
- **THEN** the provider reports support for services with none declared,
  distinguishable from a provider that has no service concept at all

### Requirement: A devenv declaration the system cannot carry is refused
Where a devenv process declares something the system cannot represent
and pass to its supervisor, `up` SHALL fail at layer `provider` naming
the process and the field it could not carry. The system SHALL NOT start
a partially translated service.

Scoped to devenv here. The general rule — that any provider's
unrepresentable declaration fails rather than being dropped — belongs to
`fix-lossy-service-translation`, and this requirement SHALL NOT be read
as having settled it for every provider. devenv is stated separately
because it is the first provider whose schema exceeds the system's
intermediate representation, so it cannot land without an answer.

#### Scenario: An unrepresentable field stops `up`
- **WHEN** a devenv process declares a field the system cannot translate
- **THEN** `up` fails at layer `provider`, naming the process and the
  field
- **AND** no service from that project is started

#### Scenario: A representable declaration is carried whole
- **WHEN** a devenv process declares a command, environment variables, a
  working directory, a restart policy, a dependency on another process,
  and a shutdown signal with a grace period
- **THEN** every one of them reaches the supervisor, and none is
  silently discarded on the way

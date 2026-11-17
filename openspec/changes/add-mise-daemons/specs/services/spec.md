## ADDED Requirements

### Requirement: Experimental mise daemon import
The mise provider SHALL read `[daemons]` from the selected manifest as data,
without invoking mise or pitchfork, and SHALL require the manifest's own
experimental opt-in. It SHALL run admitted daemons through devcroft's
keeper-owned supervisor, starting all of them on `up` unless `--skip-hooks` is
passed, and SHALL require that supervisor to be a locked tool of the project.
It SHALL NOT start or register a host pitchfork process, invoke a container
runtime, or consult the caller's environment for the opt-in.

#### Scenario: Foreground daemon
- **WHEN** an opted-in project declares a daemon with `run` and starts without `--skip-hooks`
- **THEN** the keeper starts it inside the sandbox through the existing supervisor
- **AND** no process starts during provider resolution

#### Scenario: No daemons declared
- **WHEN** a mise project has no `[daemons]` table
- **THEN** service support is reported as declared with an empty set
- **AND** that is distinguishable from a provider with no service concept

#### Scenario: Opt-in absent
- **WHEN** `[daemons]` exists and the manifest does not set the experimental opt-in
- **THEN** `up` fails at layer `provider`, exit code 3, naming the table and the setting
- **AND** an opt-in present only in the caller's environment does not count

#### Scenario: Supervisor is not a project tool
- **WHEN** daemons are declared and the supervisor is not among the locked tools
- **THEN** `up` fails at layer `provider`, exit code 3, naming it with the commands that add and lock it
- **AND** no host binary is substituted

### Requirement: Admitted mise daemon fields keep their meaning
The adapter SHALL carry `run` in string and array form, `dir`, `env`, local
`depends`, a single fixed `port` as the `PORT` variable, `init`, and an integer
`retry`. `init` SHALL run before the process on every start and restart, under
the runtime policy, and a failing `init` SHALL prevent that start. Dependency
cycles and unknown targets SHALL fail before any service starts.

#### Scenario: Array command
- **WHEN** `run` is an array whose elements contain spaces or shell metacharacters
- **THEN** the process receives exactly those elements as its arguments

#### Scenario: Initialization on restart
- **WHEN** a daemon with `init` is restarted by its restart policy
- **THEN** `init` runs again before the process, without provisioning privileges

#### Scenario: Initialization fails
- **WHEN** `init` exits non-zero
- **THEN** the process is not started for that attempt and the service is reported as failed

#### Scenario: Invalid graph
- **WHEN** `depends` contains a cycle or names an undeclared daemon
- **THEN** `up` fails at layer `provider`, exit code 3, before any service starts

### Requirement: TCP readiness is probed by devcroft itself
The system SHALL support a typed TCP readiness probe that does not depend on
any program in the project's environment and is not an HTTP check. For mise it
SHALL carry `ready_port` as that probe, `ready_cmd` as a command probe and
`ready_http` as an HTTP probe. A dependent SHALL wait for a target's readiness
where the target declares a check. A probe that never passes SHALL NOT block
`up` or tear the sandbox down. The per-check timeout SHALL be refused by its
dotted key.

#### Scenario: Dependency on a TCP-ready daemon
- **WHEN** daemon B depends on daemon A, and A declares `ready_port`
- **THEN** B starts only once A's port accepts a connection
- **AND** A's process having been spawned is not enough

#### Scenario: Environment without network tools
- **WHEN** the project's environment contains no program able to open a socket
- **THEN** the TCP probe still works

#### Scenario: Readiness never arrives
- **WHEN** a daemon's probe never passes
- **THEN** `up` completes, `status` reports `started-but-not-ready`, and its dependents do not run

#### Scenario: Per-check timeout
- **WHEN** a ready check is written in object form with a `timeout`
- **THEN** `up` fails at layer `provider`, exit code 3, naming the daemon and the dotted key
- **AND** the deadline is not approximated with attempt counts

#### Scenario: Dependency on a daemon with no ready check
- **WHEN** B depends on A and A declares no ready check
- **THEN** B waits as long as upstream's implicit ready delay would make it wait, or the edge is refused by name
- **AND** B is never started early without saying so

### Requirement: A declared shutdown grace is honoured
For every provider that declares a stop signal with a grace period, the system
SHALL give the service that long before forcing it, up to a stated maximum,
and SHALL refuse by name a declared grace above the maximum rather than
shorten it. `down` SHALL leave no service process alive whatever the grace.
Services that declare no grace SHALL keep the existing behavior.

#### Scenario: Service needs longer than a second
- **WHEN** a service declares a grace of five seconds and takes four to stop on its signal
- **THEN** `down` lets it exit by itself

#### Scenario: Service ignores its signal
- **WHEN** a service with a declared grace never exits on its stop signal
- **THEN** it is killed once the grace has elapsed and `down` leaves no process behind

#### Scenario: Grace above the maximum
- **WHEN** a service declares a grace longer than the maximum
- **THEN** `up` fails at layer `provider`, exit code 3, naming the service and the key

#### Scenario: Another provider's declared grace
- **WHEN** a devenv process declares a grace above one second
- **THEN** it is honoured the same way

### Requirement: mise daemon translation refuses lost behavior
The adapter SHALL refuse, by daemon and dotted key, every key that declares
something it does not carry, including nested and unknown keys. It SHALL refuse
presets in shorthand and table form, task daemons, container images,
references to other projects and to shared providers, unbounded retry,
automatic and multiple ports, a daemon that opts out of the project
environment, liveness checks, and any proxy setting other than disabling it.
An upstream default that devcroft does not carry SHALL be documented.

#### Scenario: Preset
- **WHEN** a manifest declares `postgres = "18"` or a daemon with `preset`
- **THEN** `up` fails at layer `provider`, exit code 3, naming the daemon and stating that presets are not supported
- **AND** nothing is installed

#### Scenario: Unknown nested key
- **WHEN** an admitted daemon carries a non-empty key the adapter does not know
- **THEN** `up` fails naming the exact dotted key

#### Scenario: Reference to a shared database
- **WHEN** a daemon names another project or a shared provider
- **THEN** the adapter refuses it without connecting to or registering anything

### Requirement: mise services obey existing policy and lifecycle ownership
Imported daemons SHALL use the existing network and service-port controls and
SHALL NOT widen any grant. Where a port is allocated for a daemon, a TCP probe
on that daemon's declared port SHALL follow the allocation, and a probe on any
other port SHALL be refused. The keeper SHALL own logs, status, partial-start
cleanup and shutdown. Detection of mise daemons in a project that selected a
different provider SHALL follow the existing cross-provider rule: it applies
only when the selected provider has no service concept, and never under
`--skip-hooks`.

#### Scenario: Undeclared port
- **WHEN** a daemon needs a port the manifest's network configuration does not permit
- **THEN** the existing policy refuses it and no grant is added

#### Scenario: Allocated port and its probe
- **WHEN** two sandboxes sharing loopback run the same daemon with an allocated `PORT` and a `ready_port` equal to the declared port
- **THEN** each probes its own allocated port

#### Scenario: Project selects a provider with services
- **WHEN** a flox project also contains a mise manifest with `[daemons]`
- **THEN** `up` is not refused because of them

#### Scenario: Project selects a provider without services
- **WHEN** a nix project contains a mise manifest with `[daemons]` and starts without `--skip-hooks`
- **THEN** `up` fails at layer `provider`, exit code 3, naming them and the selected provider

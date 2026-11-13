## ADDED Requirements

### Requirement: Service-backed VM sandboxes run catalog services in a private companion VM

When a VM sandbox requests one or more catalog service profiles, the runtime
SHALL launch a separate service companion microVM (`VM companion`) paired with
that sandbox's workload microVM (`VM workload`), each in its own confined VMM
helper with its own guest kernel, memory and private writable disk. When a VM
sandbox requests no catalog profile, the runtime SHALL NOT launch a companion VM.

#### Scenario: Sandbox requests the PostgreSQL and Redis profiles

- **WHEN** `up` launches a VM sandbox that requests both profiles
- **THEN** the workload VM and the companion VM start in separate confined helpers
- **AND** guest root in the workload VM has no device, mapping or descriptor that reaches the companion's memory or writable disk

#### Scenario: Sandbox requests no catalog profile

- **WHEN** `up` launches a VM sandbox with no catalog profile requested
- **THEN** only the workload VM is launched
- **AND** no companion helper, companion supervisor, broker, socket directory or writable service disk is allocated

### Requirement: Only trusted catalog profiles run in the companion

A companion SHALL run only service profiles from the catalog verified against
the user-trusted distribution manifest. A profile SHALL fix the service closure,
configuration, durability settings, client role and authentication rules, stop
signal, readiness probe, data format version and default disk size. The
repository SHALL be able to request a profile by name but SHALL NOT supply or
alter one. Provider-declared service commands SHALL NOT run in the companion.

#### Scenario: Project declares a flox service with its own command

- **WHEN** a VM project's provider declares a service whose command is project shell
- **THEN** that service does not run in the companion, whatever its name
- **AND** it runs where `add-vm-project-runtime` runs provider services, or is refused by name

#### Scenario: Repository ships its own service profile

- **WHEN** the repository supplies a profile definition or a closure claiming to be a catalog entry
- **THEN** the request is refused at layer `provider` unless the entry verifies against the user-trusted distribution manifest

#### Scenario: Project requests an unknown profile

- **WHEN** `devcroft.toml` requests a catalog profile name the catalog does not contain
- **THEN** `up` fails at layer `config` (exit 2), naming the profile, before any VM starts

### Requirement: The companion guest starts only catalog services and offers no login

The companion SHALL boot the pinned Linux LTS kernel and the verified read-only
image of the requested profiles, start only those services and the relay
forwarder under dedicated unprivileged identities, and provide no login daemon,
interactive shell or general-purpose remote administration.

#### Scenario: Companion boots the PostgreSQL profile

- **WHEN** the companion VM boots its verified kernel and service image
- **THEN** its init mounts the read-only image and the private writable disk and starts only the requested profiles and the relay forwarder
- **AND** no login daemon, SSH server, keeper or interactive shell is present

### Requirement: Declared service ports are relayed on loopback through a per-instance broker

The workload VM SHALL reach each requested profile on `127.0.0.1:<port>` through
a workload relay listener, the instance's own host broker and a companion relay
forwarder that connects to `127.0.0.1:<port>` in the paired companion. The
workload listener SHALL be running before creation-time setup and after every
workload boot. The broker SHALL carry one host stream per TCP connection, SHALL
enforce a finite concurrent-connection cap and per-direction buffer recorded in
the launch plan, SHALL preserve half-close, and SHALL be launched only with its
own instance's endpoints. An undeclared port SHALL have no mapping. Neither VM
SHALL have a general network interface (`NIC` or `TSI`), and the companion SHALL
have no guest-initiated mapping toward the host or the workload.

#### Scenario: Workload connects to the declared PostgreSQL port

- **WHEN** a workload process connects to `127.0.0.1:5432`
- **THEN** the broker carries that connection as its own stream to `127.0.0.1:5432` in the paired companion, preserving half-close
- **AND** two concurrent sandboxes both use port `5432` without host port collision

#### Scenario: Workload probes an undeclared companion port

- **WHEN** a workload process requests a port that no requested profile declares
- **THEN** no relay mapping exists for it and nothing reaches the companion

#### Scenario: Workload exceeds the connection cap

- **WHEN** the workload opens more concurrent connections than the recorded cap
- **THEN** the broker refuses the excess connections and existing streams continue

#### Scenario: Compromised companion attempts to reach the workload or the host

- **WHEN** a process in the companion tries to open a connection toward the workload or the host
- **THEN** no guest-initiated mapping exists in the companion's launch inventory and the attempt fails

#### Scenario: A broker is started with another instance's endpoint

- **WHEN** a broker launch names a workload or companion socket outside its own instance's controller record
- **THEN** the launch is refused and no stream is carried

#### Scenario: Workload reboots after setup

- **WHEN** the workload reboots at the end of creation-time setup
- **THEN** streams opened before the reboot are closed at the broker
- **AND** the workload listener is running again before sessions are admitted

### Requirement: Companion readiness is probed by the host before workload setup and agent admission

Before running workload creation-time setup and before admitting interactive or
fleet sessions, the broker SHALL run each requested profile's readiness probe
through the declared port. A probe that misses its deadline SHALL fail launch at
layer `keeper`, before setup runs.

#### Scenario: Database migration runs during workload setup

- **WHEN** `up` starts a dual-VM sandbox whose setup hook migrates PostgreSQL
- **THEN** workload setup starts only after the broker's PostgreSQL probe passes
- **AND** a probe timeout fails `up` at layer `keeper` (exit 5) before setup runs

### Requirement: Companion failures keep the five-layer error contract

Companion failures SHALL name an existing layer: `config` for catalog profile
requests, `provider` for catalog verification and data format version, `backend`
for the companion helper, VMM and host confinement, and `keeper` for companion
PID 1, its control and diagnostic channels, the companion supervisor, the broker
and readiness.

#### Scenario: Companion helper fails host confinement

- **WHEN** the companion helper cannot apply its host confinement before the first guest instruction
- **THEN** `up` fails at layer `backend` (exit 4) and the workload is not admitted

### Requirement: Graceful stop closes client streams and flushes services before helper exit

Normal shutdown (`down` or restart) SHALL stop workload sessions, close every
relay stream at the broker, send `SHUTDOWN` to companion PID 1, let PID 1 stop
each service with its profile's stop signal, synchronize filesystems, unmount the
service disk and acknowledge completion. A stop SHALL be classified as orderly
only when that acknowledgement arrives, correlated with the controller's
recorded stop intent and the helper's recorded identity; the helper's exit
status SHALL NOT count as evidence. Otherwise it SHALL be recorded as a forced
stop or crash, and the next launch SHALL check the disk and recover services
before the relay reopens.

#### Scenario: Normal down with client connections still open

- **WHEN** `devcroft down` runs while workload background processes hold PostgreSQL connections
- **THEN** the broker closes those streams before `SHUTDOWN`, PostgreSQL stops in Fast Shutdown and PID 1 acknowledges completion
- **AND** no forced stop is recorded and the next `up` needs no service crash recovery

#### Scenario: Helper exits with status zero without an acknowledgement

- **WHEN** the companion helper exits with status 0 and no `SHUTDOWN` acknowledgement was received
- **THEN** status records a companion crash, not an orderly stop

#### Scenario: Companion helper is killed during Redis and PostgreSQL writes

- **WHEN** the companion helper receives `SIGKILL` during active writes
- **THEN** status records a forced companion stop
- **AND** the next `up` runs a pre-mount filesystem check and recovers every write acknowledged under the profile's durability settings (PostgreSQL commits with `synchronous_commit=on`, Redis writes with `appendfsync always`) without affecting sibling sandboxes
- **AND** no claim is made for host power loss

### Requirement: A crashed companion restarts automatically through recovery

A per-instance companion supervisor, started by `up` with the instance's
lifetime, SHALL restart the companion automatically when its helper exits
without an acknowledged `SHUTDOWN` or PID 1 stops answering while the workload
runs. During a restart, the broker SHALL refuse new relay streams and close open
ones, and `status` SHALL show the companion as restarting. Each restart SHALL run
the pre-mount check and service recovery, and the relay SHALL reopen only after
the readiness probe passes. Restarts SHALL back off, and SHALL stop after a
bounded number of consecutive failures within a window recorded in the launch
plan, leaving `status` reporting the companion as crashed with the last reason.
The supervisor SHALL NOT restart the companion after a recorded stop request, or
after an incompatible data format version, a filesystem check that needs manual
repair, a catalog verification failure or a host confinement failure. The
supervisor SHALL NOT carry relay streams.

#### Scenario: Companion kernel panics during a session

- **WHEN** the companion guest panics while workload sessions are active, and the helper exits with status 0
- **THEN** relayed connections close, `status` shows the companion as restarting and the workload keeps running
- **AND** the supervisor relaunches the companion, recovery runs, and the relay reopens after the readiness probe passes

#### Scenario: The companion keeps crashing

- **WHEN** consecutive restarts fail more often than the recorded bound within its window
- **THEN** the supervisor stops restarting and `status` reports the companion as crashed with the last reason
- **AND** the next `up` or explicit restart tries again

#### Scenario: The disk needs manual repair

- **WHEN** the pre-mount filesystem check after a crash reports damage it cannot repair automatically
- **THEN** the companion stays stopped without further restarts and `status` names the reason

#### Scenario: Stop is requested while the companion is restarting

- **WHEN** `devcroft down` records a stop request during a restart
- **THEN** the supervisor makes no further restart attempt and stops the companion

### Requirement: Persistent service data is bounded and bound to its data format version

The companion's writable disk SHALL have a finite size and SHALL record the
profile and data format version that wrote it. A full disk SHALL be reported in
`status` as a companion service failure. A launch that selects a profile with an
incompatible data format version SHALL refuse at layer `provider` by name, leave
the data untouched and SHALL NOT upgrade it implicitly.

#### Scenario: Catalog update moves PostgreSQL to a new major version

- **WHEN** an updated catalog entry selects a PostgreSQL major version different from the one that wrote `companion-rw.img`
- **THEN** `up` refuses at layer `provider`, naming both versions and stating that devcroft performs no upgrade
- **AND** the existing data directory is not modified

#### Scenario: Service disk fills

- **WHEN** the service data reaches the size of `companion-rw.img`
- **THEN** `status` reports the companion disk as full and names the failing service
- **AND** the disk does not grow beyond its recorded size

### Requirement: Dual-VM resource budgets and fleet admission account for both VMs

Launch plans, `status` and fleet admission SHALL account for both helpers, both
guests, the broker, the companion supervisor and the companion's private disk,
reporting workload and
companion separately and together. The per-environment storage ceiling SHALL be
checked against the companion's fixed footprint, with service data bounded by
the disk size. Budgets SHALL be checked against the owner's recorded thresholds
without reclassifying retained artifacts.

#### Scenario: Fleet admission checks host capacity for dual-VM sandboxes

- **WHEN** a fleet of `N` service-backed sandboxes is requested
- **THEN** admission evaluates host memory, CPU and storage against `2N` microVMs, including each companion disk's recorded size
- **AND** refuses admission before launch if the combined budget exceeds configured host limits

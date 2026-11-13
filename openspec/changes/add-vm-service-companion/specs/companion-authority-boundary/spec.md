## ADDED Requirements

### Requirement: The companion VM has zero outbound egress, zero shell and no control-plane API

The companion VM SHALL have no outbound internet or proxy channel, no DNS
resolver, no SSH server, no keeper, no interactive shell, no project source or
workspace mounts, and no access to devcroft host control-plane APIs.

#### Scenario: Service process inside the companion attempts outbound network access

- **WHEN** code running inside the companion VM attempts an external TCP, UDP or DNS connection
- **THEN** no network interface, TSI or egress proxy channel exists in the companion VM and the operation fails

#### Scenario: Operator or workload attempts to open a shell in the companion VM

- **WHEN** a client requests an interactive shell or `exec` session targeting the companion VM
- **THEN** the request is refused because the companion exposes no keeper or SSH endpoint

### Requirement: The companion's host channels are a closed, host-initiated inventory

The companion's launch inventory SHALL contain only the per-port service relay
mappings, a `SHUTDOWN` control channel and a diagnostic stream, each
host-initiated. The control channel SHALL accept connections only from the host
and exactly one opcode, `SHUTDOWN`, bound to the instance generation, which is
delivered at boot as a non-secret value, with a fixed frame cap. PID 1 SHALL hold
it, bound before any service starts. Unknown opcodes, oversized frames, other
peers and generation mismatches SHALL close the connection. The two non-relay
channels are an owner-approved exception (2026-11-13) to the rule that the
companion has no controller API and no direct host collector; adding to them
SHALL require a new owner decision.

#### Scenario: Launch inventory is inspected

- **WHEN** the controller renders a running companion's launch plan
- **THEN** it lists the relay mappings, the `SHUTDOWN` control and the diagnostic mapping, all host-initiated
- **AND** it lists no guest-initiated mapping

#### Scenario: Host sends an opcode other than `SHUTDOWN`

- **WHEN** a frame on the control channel carries any other opcode, such as a secret or configuration payload
- **THEN** PID 1 closes the connection without acting on it

#### Scenario: Service process attempts to take over or use the control channel

- **WHEN** a process running as the `postgres` or `redis` identity tries to bind the control port, connect to it over local vsock or reach PID 1's state
- **THEN** the port is already bound by PID 1, PID 1 closes any connection whose peer is not the host, and PID 1's state directory (`0700`, `root:root`) is inaccessible to it

### Requirement: Instance identity comes from the broker channel and only the client role is reachable

The companion SHALL rely on the per-instance broker for instance identity and
SHALL NOT receive per-instance secrets. Each catalog profile SHALL give the
workload a client role that cannot execute programs or load code on the server
and cannot change protected server configuration, and SHALL make every other
server identity unreachable through the relay. Administrative setup SHALL run
from the catalog at first boot over the companion-local Unix socket, not through
the relay.

#### Scenario: Workload connects as an administrative identity

- **WHEN** the workload connects through the relay as the PostgreSQL superuser or another non-client role
- **THEN** `pg_hba.conf` rejects the connection, because administrative roles authenticate only with `peer` on the companion-local Unix socket

#### Scenario: Workload tries to become another Redis user

- **WHEN** the workload issues `AUTH` as any user other than the client user, or `ACL SETUSER`
- **THEN** no other user exists to authenticate as and the `ACL` command is refused

#### Scenario: Workload attempts server-side program execution through PostgreSQL

- **WHEN** the workload's role issues `COPY ... FROM PROGRAM`, creates an untrusted-language function or runs `ALTER SYSTEM`
- **THEN** PostgreSQL refuses for lack of privilege

#### Scenario: Workload attempts to reconfigure Redis

- **WHEN** the workload's Redis user issues `CONFIG SET dir`, `CONFIG SET appendfsync`, `MODULE LOAD` or `DEBUG`
- **THEN** the ACL or protected-config setting refuses the command

#### Scenario: Images and logs are inspected for secrets

- **WHEN** the shared image, `/proc/cmdline`, controller state and host logs are inspected
- **THEN** none contains a per-instance credential, because none is delivered

### Requirement: The companion's protection claim states its limits

Documentation, `status` and the threat model SHALL describe the companion as
protecting its kernel, memory, service binaries, configuration, administrative
identities, raw disk and service lifecycle from the paired workload. They SHALL
NOT claim that it protects data reachable through the granted client role,
prevents exfiltration through service responses, or keeps service log content
authentic.

#### Scenario: Workload deletes data through its granted role

- **WHEN** a compromised workload drops a table it owns through the relayed connection
- **THEN** the deletion succeeds, as the published limits state
- **AND** no devcroft output claims the companion would have prevented it

### Requirement: Helpers and brokers confine which Unix sockets each role can reach

Every VMM helper (workload, companion, preparation) SHALL apply host confinement
in-process after opening its read-only and private read-write descriptors and
before the VMM is built and the first guest instruction runs. Each broker and
each companion supervisor SHALL apply the same platform controls before
accepting a stream or launching a helper. Confinement SHALL limit which Unix
sockets a role can connect to, not only which it can create or remove: the
workload helper SHALL reach only its own sockets and the broker's relay
listeners, the broker only its listeners and the companion's relay mappings, the
companion helper only its own sockets, only the companion supervisor the
companion's control and diagnostic sockets, and only the controller the
supervisor's request socket and `meta/`. The companion supervisor's confinement
SHALL cover the companion helper's grants and nothing on the workload side. On macOS, confinement
SHALL use a deny-default Seatbelt profile that limits outbound Unix-socket
connections by path. On Linux, it SHALL use Landlock ABI ≥ 6, a `seccompiler`
kill-on-miss syscall allowlist in force on every VMM thread, and a per-role
mount namespace or separate host UID, because Landlock ABI 6 and seccomp do not
mediate `connect()` to a pathname socket. Production adoption SHALL wait for the
owner's `author_claimed_security_boundary` decision recorded under
`evaluate-microvm-runtimes` task 4.5.

#### Scenario: Linux helper is launched with Landlock but without seccomp

- **WHEN** a Linux helper reaches the first guest instruction without a seccomp syscall filter in force
- **THEN** qualification and launch fail closed

#### Scenario: Linux helper is launched without socket-reach separation

- **WHEN** a Linux helper starts without a per-role mount namespace or separate host UID
- **THEN** launch fails closed at layer `backend` instead of claiming socket separation

#### Scenario: Compromised workload helper reaches for the companion's sockets

- **WHEN** a workload helper compromised through a VMM escape tries to connect to the companion's control, diagnostic or relay sockets
- **THEN** host confinement denies the connection, and the only companion-bound path it can reach is the broker's relay listeners

#### Scenario: Confined helper attempts to modify controller metadata

- **WHEN** a compromised helper attempts to open or overwrite the instance spec or lease records in `meta/`
- **THEN** host confinement denies access because `meta/` is outside its reach

#### Scenario: Compromised companion helper targets the workload's sockets

- **WHEN** a compromised companion helper tries to unlink, bind or connect to a socket in the workload's socket directory
- **THEN** host confinement denies the operation because that directory is outside its reach

### Requirement: Companion diagnostics bypass the workload and optional eBPF is non-gating

Companion PID 1 SHALL write service output and its own structured lifecycle
events to the host-initiated diagnostic stream, which the host stores in a
size-bounded buffer without passing through the workload. The companion
supervisor SHALL hold the only diagnostic connection, opened before any service
starts on every boot. Service
output SHALL be labelled distinctly from PID 1 events. PID 1 SHALL reserve
capacity for its own events, SHALL drop only service output under pressure, SHALL
report drop counts as PID 1 events and SHALL NOT block on the stream. Optional
in-guest eBPF or Falco monitoring SHALL NOT be required for launch and SHALL NOT
be claimed as a security boundary.

#### Scenario: Workload VM is compromised by root-level malware

- **WHEN** the workload VM attempts to suppress or forge companion PID 1 events
- **THEN** it has no mapping to the diagnostic stream and host-collected PID 1 events remain intact
- **AND** service log lines the workload induced through the protocol are labelled as service output

#### Scenario: The workload makes a service flood its output

- **WHEN** relayed requests make a service write faster than the host drains the diagnostic stream
- **THEN** PID 1 drops only service output, its own events and the drop counts reach the host, and it still completes `SHUTDOWN`

#### Scenario: Diagnostic connection cannot be opened

- **WHEN** the companion supervisor cannot open the diagnostic connection, or PID 1 reports it already held
- **THEN** launch fails at layer `keeper` before any service starts

#### Scenario: Optional Linux eBPF diagnostic profile is disabled by default

- **WHEN** a sandbox launches with the default minimal production kernel
- **THEN** launch succeeds without `CONFIG_BPF_SYSCALL` or Falco in either VM
- **AND** enabling eBPF/Falco requires explicitly selecting the separate diagnostic kernel profile

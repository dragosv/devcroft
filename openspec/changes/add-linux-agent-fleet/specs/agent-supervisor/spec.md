# agent-supervisor

## ADDED Requirements

### Requirement: Supervisor owns agent process lifecycle

The supervisor SHALL create every agent process itself, via `clone`/`fork`
followed by re-exec of an internal init helper. It SHALL NOT delegate process
creation to the sandbox library or to an external sandboxing binary.

#### Scenario: Agent is started

- **WHEN** an agent is started
- **THEN** the supervisor allocates an agent ID, creates the agent's cgroup,
  builds the sandbox ruleset, and clones a child that re-execs as the init helper
- **AND** the supervisor retains a handle capable of querying status and
  terminating the agent

#### Scenario: Sandbox library exposes only a combined spawn entry point

- **WHEN** the pinned sandbox crate offers no way to build a ruleset separately
  from applying it to the current process
- **THEN** the supervisor SHALL fail at build or startup with a diagnostic
  naming the missing capability
- **AND** the fleet feature SHALL NOT silently degrade to unsandboxed execution

### Requirement: Init helper is single-threaded and re-executed

The supervisor SHALL apply namespace, mount and sandbox setup in a re-executed
single-threaded helper process, never in the forked child of a multi-threaded
runtime.

#### Scenario: Child performs setup

- **WHEN** the cloned child begins execution
- **THEN** it immediately re-execs the devcroft binary as the internal init
  subcommand
- **AND** the helper receives its configuration over an inherited pipe and its
  ruleset over an inherited file descriptor
- **AND** the helper performs, in order: enter namespaces, perform mounts, add
  namespace-local sandbox rules, apply the sandbox ruleset, apply the syscall
  filter if configured, then exec the agent command

#### Scenario: Setup fails inside the helper

- **WHEN** any setup step in the init helper fails
- **THEN** the helper SHALL exit without exec'ing the agent command
- **AND** the failing step and its errno SHALL be reported to the supervisor over
  the configuration pipe

### Requirement: Agents are individually addressable

The supervisor SHALL expose commands to list running agents, inspect one agent,
and stop one agent, each identified by a stable agent ID.

#### Scenario: Listing agents

- **WHEN** the operator lists agents
- **THEN** each entry reports agent ID, state, workspace path, current memory
  usage, accumulated CPU time, and any host port mappings

#### Scenario: Stopping one agent of many

- **WHEN** the operator stops a single agent while others are running
- **THEN** only that agent's process tree is terminated
- **AND** other agents remain running and unaffected

### Requirement: Preflight environment validation

The supervisor SHALL validate host prerequisites before starting the first agent
and SHALL report precisely which prerequisite is missing.

#### Scenario: Unsupported host

- **WHEN** cgroup v2 delegation, unprivileged user namespaces, or the required
  kernel ABI level is unavailable
- **THEN** the supervisor SHALL refuse to start the fleet
- **AND** SHALL name the specific missing prerequisite and the remediation where
  one exists

#### Scenario: Non-Linux host

- **WHEN** fleet commands are invoked on macOS
- **THEN** the supervisor SHALL report that fleet is Linux-only and point to the
  VM-based path

### Requirement: Per-agent endpoints are created before restriction

Each agent SHALL have its own control and SSH sockets, created by the
supervisor before that agent's restriction is applied, with mode 0600 inside a
0700 state directory.

This is the existing listener-before-restriction ordering applied per agent
rather than per sandbox: Landlock and seccomp are inherited by children and
cannot be joined from outside, so a socket that does not predate the
restriction is unreachable for the agent's lifetime. The filesystem permissions
remain the real access boundary — the same as for a single sandbox, and for the
same reason.

#### Scenario: Reaching one agent

- **WHEN** a client connects to a specific agent's SSH endpoint
- **THEN** it reaches that agent and no other
- **AND** the socket's permissions, not its location, are what restrict access

#### Scenario: Socket creation fails

- **WHEN** an agent's sockets cannot be created
- **THEN** that agent does not start
- **AND** the failure does not leave a partially-started agent whose endpoints
  are unreachable

### Requirement: Fleet state records durable identity separately from runtime facts

Fleet SHALL persist, per agent, a stable identity: its workspace, its cgroup
path, its lifecycle state, its policy fingerprint, its port mappings, and
whether that agent needs attention (`add-agent-interaction`). Facts
reconstructible after a crash SHALL NOT be persisted as though authoritative.

Attention belongs in this record rather than being added later, and the reason
is fleet-specific: an agent that stops to ask something is the normal case at
N > 1, and "which of my agents is blocked" is a listing question. A record
without it forces the answer to be a search.

The distinction matters at recovery: a supervisor restarting after a crash must
be able to tell an agent it still owns from a stale record, and must not adopt
a cgroup or a port mapping that some other process now holds. Recording a live
PID as durable identity is exactly the mistake — pids are reused, which the
single-sandbox lifecycle already had to learn.

#### Scenario: Supervisor restarts

- **WHEN** the supervisor restarts while agents are running
- **THEN** it reconciles its recorded agents against what is actually alive
- **AND** it neither adopts an agent it no longer owns nor abandons one it does

#### Scenario: A stale agent record

- **WHEN** a recorded agent's processes are gone
- **THEN** its record is reconciled rather than reported as running
- **AND** its port mappings are released for reuse

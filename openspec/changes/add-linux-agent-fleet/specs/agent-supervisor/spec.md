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

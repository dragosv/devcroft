# agent-supervisor

## ADDED Requirements

### Requirement: Supervisor owns agent process lifecycle

The supervisor SHALL create every agent process itself, via `clone`/`fork`
followed by re-exec of an internal init helper. It SHALL NOT delegate process
creation to the sandbox library or to an external sandboxing binary.

#### Scenario: Agent is started

- **WHEN** an agent is started
- **THEN** the supervisor allocates an agent ID, creates the agent's cgroup,
  compiles the agent's policy, and clones a child directly into that cgroup
  that re-execs as the init helper
- **AND** the supervisor retains a handle capable of querying status and
  terminating the agent

#### Scenario: The sandbox cannot be applied

- **WHEN** the init helper cannot apply the agent's policy
- **THEN** the agent does not start, and the supervisor reports the failing
  step
- **AND** the fleet feature SHALL NOT silently degrade to unsandboxed execution

#### Scenario: An agent exits with host-side helpers running

- **WHEN** an agent that has host-side helpers (its egress proxy, its port
  forwarders) exits, however it exits, including a crash no fleet command
  observes
- **THEN** those helpers exit with it, without waiting for a later command
  to reconcile the agent's record

#### Scenario: A start is interrupted after its helpers are launched

- **WHEN** the supervisor dies after launching an agent's host-side helpers
  and before that agent is recorded
- **THEN** the helpers exit rather than running on with no record naming
  them
- **AND** the next command removes what is left of the start, host-side
  helpers' cgroup included

### Requirement: Init helper is single-threaded and re-executed

The supervisor SHALL apply namespace, mount and sandbox setup in a re-executed
single-threaded helper process, never in the forked child of a multi-threaded
runtime.

#### Scenario: Child performs setup

- **WHEN** the cloned child begins execution
- **THEN** it immediately re-execs the devcroft binary as the internal init
  subcommand
- **AND** the child does not exec until the supervisor has written its
  identity map, since an exec by an unmapped uid drops every capability
- **AND** the helper receives its configuration, including the compiled
  policy, over an inherited pipe
- **AND** the helper performs, in order: perform mounts, build the sandbox
  ruleset from the policy and apply it, apply the syscall filter if
  configured, then start the agent command as its child

The ruleset is built inside the helper, after the mounts, and not carried
across the clone. The view's `/tmp`, `/dev/pts` and `/proc` are new
filesystem instances that do not exist in the supervisor, and Landlock rules
bind to inodes, so rules prepared beforehand attach to the host's instances
(measured: they refused the agent's private `/tmp`). The helper is a freshly
exec'd, single-threaded process, so building it there has none of the
post-`fork` allocation hazard that motivated preparing it in the parent.

#### Scenario: Setup fails inside the helper

- **WHEN** any setup step in the init helper fails
- **THEN** the helper SHALL exit without exec'ing the agent command
- **AND** the failing step and its errno SHALL be reported to the supervisor over
  a status pipe the helper inherits for that purpose

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

#### Scenario: An ID is never reused

- **WHEN** an agent is removed, including the most recently started one, and
  another agent is started in the same fleet, before or after a supervisor
  restart
- **THEN** the new agent gets an ID no agent of that fleet has had
- **AND** removing the whole fleet ends that fleet, so the IDs of a fleet
  started afterwards are its own

#### Scenario: Reaching an agent by name

- **WHEN** an SSH client connects to `<id>.<sandbox name>.devcroft` through
  the `ProxyCommand` devcroft's SSH configuration installs
- **THEN** it reaches that agent's own SSH endpoint
- **AND** an agent that is not running is reported as such, by ID and
  fleet, and is never started by the connection attempt

### Requirement: Fleet commands are serialized per fleet, and a fleet belongs to one project

Every fleet command that reads or changes agent state SHALL hold a per-fleet
lock for the whole of that work, released by the kernel however the command
exits. A fleet SHALL record the project it was started for, and every
command SHALL refuse a fleet recorded for a different project before
changing anything.

Fleet state is keyed by sandbox name, and two checkouts of one repository
share a committed manifest and so a name. Without the project check, the
second checkout's commands act on the first one's agents; without the lock,
two commands interleave, and one command's half-finished start is
indistinguishable from a crashed one to the other.

#### Scenario: A command runs while an agent is starting

- **WHEN** a fleet command runs while another is starting an agent in the
  same fleet
- **THEN** it waits for that start to finish, saying that it is waiting
- **AND** it never treats the agent being started as a crashed start

#### Scenario: Two starts at once

- **WHEN** two `fleet up` commands run at once for the same fleet
- **THEN** both succeed and their agents have distinct IDs

#### Scenario: Another checkout with the same sandbox name

- **WHEN** a fleet command runs in a project other than the one the fleet
  was started for
- **THEN** it fails with a configuration error naming both projects
- **AND** it changes no fleet state, no agent and no clone

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

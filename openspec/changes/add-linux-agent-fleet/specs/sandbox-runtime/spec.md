# sandbox-runtime Delta Specification (add-linux-agent-fleet)

## Purpose

Constructing the world one agent runs in: namespaces, identity, the filesystem
view, and the ordering that makes the restriction reliable rather than
best-effort.

The ordering is the substance. Every requirement here exists because there is
exactly one sequence in which namespace entry, identity mapping, mounts,
restriction and workload start can happen without leaving a window where
project-supplied code runs unrestricted.

## ADDED Requirements

### Requirement: Each agent gets a full set of rootless namespaces

Each agent SHALL run in its own user, mount, PID, network, IPC and UTS
namespaces, created without privilege on the host. Fleet SHALL NOT fall back to
sharing the host's namespaces for any of them.

#### Scenario: Agents cannot observe each other

- **WHEN** two agents are running
- **THEN** neither can enumerate or signal the other's processes
- **AND** neither can observe the other's network interfaces or listeners

#### Scenario: Namespaces are unavailable

- **WHEN** the host cannot create unprivileged user namespaces
- **THEN** fleet refuses to start, naming the restriction
- **AND** it does not start agents sharing host namespaces instead

### Requirement: Identity is mapped before anything else runs

The parent SHALL write single-entry UID and GID maps for the child, with
`setgroups` denied before the GID map is written, and the child SHALL block
until this completes before performing mounts or executing any workload.

Files an agent creates in its workspace SHALL be owned by the real host user, so
the resulting clone is reviewable and committable without ownership repair.

#### Scenario: Agent writes a file

- **WHEN** an agent creates or modifies a file in its workspace
- **THEN** the file is owned by the invoking host user
- **AND** no subsequent ownership fix-up is required to inspect or commit it

#### Scenario: Mapping fails

- **WHEN** the identity mapping cannot be written
- **THEN** the child does not proceed to mounts or workload start
- **AND** the failure is reported as setup, distinguishable from the workload
  failing

### Requirement: The init helper is PID 1 and applies the restriction after mounts

The re-executed init helper SHALL be PID 1 of the agent's PID namespace, SHALL
reap descendants for the agent's lifetime, and SHALL apply the prepared sandbox
policy **after** constructing the filesystem view and **before** starting the
keeper.

The helper SHALL NOT replace itself with the workload: if PID 1 exits, the
kernel terminates the entire PID namespace.

A seam SHALL remain between applying the sandbox policy and starting the
workload, so later syscall filtering has a defined insertion point.

#### Scenario: Ordering within the helper

- **WHEN** an agent starts
- **THEN** identity mapping, then mounts, then policy application, then keeper
  start occur in that order
- **AND** no project-supplied code executes before the policy is successfully
  applied

#### Scenario: An agent process is orphaned

- **WHEN** a process inside an agent exits leaving orphaned children
- **THEN** they are re-parented to the init helper and reaped
- **AND** the agent's namespace does not accumulate zombies

#### Scenario: Restriction fails

- **WHEN** applying the sandbox policy fails
- **THEN** the agent does not start
- **AND** no workload runs unrestricted as a consequence

### Requirement: The agent's workspace view is private and complete

Each agent SHALL see its own clone read-write at a fixed path, its provider's
resolved runtime paths read-only, and a private `/tmp`, `/proc` and `/dev`.
Neither the shared source checkout nor the host path of the agent's clone SHALL
be mounted separately.

#### Scenario: Agent works in its workspace

- **WHEN** an agent builds and tests
- **THEN** its workspace is writable and its toolchain readable
- **AND** temporary files, process listings and device nodes are its own

#### Scenario: Runtime paths are not writable

- **WHEN** an agent attempts to modify a provider runtime path
- **THEN** the write is denied
- **AND** other agents sharing those resolved paths are unaffected

### Requirement: The mount-view strategy is explicit and never silently downgraded

Fleet SHALL record which mount-view strategy is in force and report it. It
SHALL NOT fall back from a more restrictive strategy to a less restrictive one
without saying so.

Both strategies preserve the same access boundary; they differ in whether
ungranted paths are *visible*. That difference is undetectable from inside a
correctly-configured agent — both deny the same reads — which is exactly why a
silent downgrade would be undetectable by the person relying on it.

#### Scenario: Operator inspects an agent

- **WHEN** the operator inspects a running agent
- **THEN** the mount-view strategy in force is reported
- **AND** it can be distinguished from the other strategy without inference

#### Scenario: The stricter strategy cannot be constructed

- **WHEN** the minimal-root view cannot be built on this host or for this
  provider
- **THEN** the failure or the fallback is reported explicitly
- **AND** the agent does not silently run under the more permissive view

# resource-control Delta Specification (add-linux-agent-fleet)

## Purpose

Keeping one agent's resource use from becoming every other agent's problem, and
making teardown atomic rather than a hunt for orphans.

This is the foundation of the change rather than a refinement of it: at N ≥ 3
the single most likely failure is one runaway build starving the host, and
Landlock has nothing to say about CPU, memory, PIDs or IO.

## ADDED Requirements

### Requirement: A delegated cgroup v2 subtree with one leaf per agent

Fleet SHALL run under a delegated cgroup v2 subtree and SHALL create one leaf
cgroup per agent. Each agent's leaf SHALL contain that agent's init helper,
keeper, sessions, services and local forwarder.

The host-side egress proxy and the supervisor itself SHALL NOT be in any
agent's leaf. The proxy sits outside the agent's policy domain by design
(design D4); placing it inside the agent's cgroup would let the agent's own
resource pressure — or its teardown — take down the component filtering it.

The internal node the leaves hang from SHALL contain no processes, because
cgroup v2's no-internal-process rule prevents a populated internal cgroup from
distributing domain controllers to its children.

#### Scenario: Fleet starts

- **WHEN** fleet starts on a host with a delegated subtree
- **THEN** an empty internal node is created, with one leaf per agent beneath it
- **AND** the supervisor and each agent's proxy remain outside the leaves

#### Scenario: Delegation is unavailable

- **WHEN** no delegated cgroup v2 subtree is available
- **THEN** fleet refuses to start, naming what is missing
- **AND** it does not fall back to running agents without resource control

### Requirement: Each agent's resources are controlled independently

Memory, CPU weight, PID count and — where available — IO weight SHALL be
controlled per agent. Where a controller is unavailable, that SHALL be reported
as a named degraded capability rather than causing the remaining limits to be
abandoned.

#### Scenario: One agent runs a runaway build

- **WHEN** one agent saturates CPU or memory
- **THEN** other agents remain schedulable and are not OOM-killed on its behalf

#### Scenario: The IO controller is unavailable

- **WHEN** the host's cgroup subtree has no `io` controller
- **THEN** memory, CPU and PID limits still apply
- **AND** the missing IO control is named, in the same way any other
  unenforceable aspect is reported

### Requirement: Teardown is cgroup-wide and waits for it to complete

Stopping an agent SHALL terminate every process in its leaf, including
descendants that re-parented or attempted to escape their process group. The
supervisor SHALL wait for the leaf to be observably empty before removing it.

#### Scenario: An agent with orphaned descendants is stopped

- **WHEN** an agent is stopped and its processes include daemonised or
  re-parented descendants
- **THEN** all of them are terminated
- **AND** nothing survives that the supervisor believed it had stopped

#### Scenario: Removing the leaf

- **WHEN** the supervisor removes a stopped agent's leaf
- **THEN** it first waits until the leaf reports itself unpopulated
- **AND** it does not remove a leaf that still contains processes

### Requirement: Resource preconditions are probed before the first agent

Fleet SHALL verify its resource-control preconditions by exercising them, not
by inspecting versions or paths, and SHALL do so before starting any agent.

Discovery SHALL NOT assume a fixed cgroup path: the layout differs by
distribution and by whether a user manager is present.

#### Scenario: Preflight on an unsuitable host

- **WHEN** the preflight cannot create a subtree, enable the required
  controllers, move a process into a leaf, or observe the leaf emptying
- **THEN** fleet refuses to start with an actionable diagnostic
- **AND** no agent is started in a state where limits appear configured but do
  not hold

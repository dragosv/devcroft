# workspace-isolation Delta Specification (add-linux-agent-fleet)

## Purpose

Giving each agent a working tree it can change freely without corrupting
another agent's, and giving it a toolchain without giving it authority over the
machine's package store.

## ADDED Requirements

### Requirement: Each agent gets an independent clone, not a shared worktree

Each agent SHALL work in its own git clone, backed by a shared bare mirror for
object storage. Agents SHALL NOT share a git worktree or a ref namespace.

Worktrees are the obvious choice and the wrong one: they share the object store
*and* refs. `index.lock` is per-worktree but `packed-refs` is not, and git takes
locks without retrying — so one agent receives a spurious failure and, being an
agent, reacts to it.

#### Scenario: Concurrent commits

- **WHEN** several agents commit, branch and rebase concurrently
- **THEN** none observes a lock failure caused by another's activity
- **AND** each agent's refs are its own

#### Scenario: Disk cost

- **WHEN** N agents are created from one repository
- **THEN** object storage is shared through the mirror rather than duplicated N
  times

### Requirement: Mirror maintenance does not run while agents are live

Automatic garbage collection SHALL be disabled on the shared mirror and on
agent clones. Maintenance SHALL be supervisor-driven and SHALL NOT run while
any agent is active.

An agent clone references the mirror's objects rather than copying them, so
pruning the mirror can remove objects a live clone still needs — and the failure
surfaces inside the agent, long after the prune, as corruption it will try to
diagnose.

#### Scenario: Maintenance is attempted with agents running

- **WHEN** mirror maintenance is requested while agents are active
- **THEN** it does not run
- **AND** the reason is reported rather than the request being silently dropped

### Requirement: Agents receive resolved runtime paths read-only, and no package-manager authority

Each agent SHALL receive its provider's resolved runtime paths read-only — the
closure for closure-tier providers, devcroft-owned artifact paths plus explicit
host library grants for qualified artifact-tier providers.

An agent SHALL NOT receive a package-manager daemon socket, and SHALL NOT
receive a writable host-global store. A workflow that requires either SHALL be
refused, naming what it asked for.

This is the multi-agent case of `sandbox-provisioning`'s P2a/P2b, and it is
where that decision matters most: a host-global store is shared by every agent,
so authority over it granted to one agent's project code is authority over every
other agent's toolchain.

#### Scenario: Agent builds against its toolchain

- **WHEN** an agent compiles or runs tests
- **THEN** its provider's runtime paths are readable
- **AND** they are not writable, from any agent

#### Scenario: A workflow requires installing a package at runtime

- **WHEN** an agent's workflow attempts to install into the host-global store,
  or to reach a package-manager daemon
- **THEN** it is refused, naming the authority requested
- **AND** the refusal distinguishes "not granted in this MVP" from "this cannot
  ever work", since the former is a scope decision and the latter is not

# workspace-isolation Delta Specification (add-linux-agent-fleet)

## Purpose

Giving each agent a working tree it can change freely without corrupting
another agent's, and giving it a toolchain without giving it authority over the
machine's package store.

## ADDED Requirements

### Requirement: Each agent gets an independent clone, not a shared worktree

Each agent SHALL work in its own git clone holding its own copies of the
objects, sharing no file with the source repository or another clone: a
shared file (a hardlinked pack) is shared by writes too, and an agent's own
`.git` is writable to it. Agents SHALL NOT share a git worktree or a ref namespace,
and SHALL NOT push into the repository they were cloned from: their work is
integrated by fetching from their clone.

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
- **THEN** each clone's objects are its own copies, and a write through one
  clone reaches neither the source nor another clone
- **AND** the disk this costs is the accepted price of that independence

#### Scenario: An agent pushes

- **WHEN** an agent pushes to the repository it was cloned from
- **THEN** the push is refused, saying that agents do not push and that their
  work is fetched from their clone

### Requirement: An agent's clone does not depend on objects another repository can prune

An agent's clone SHALL hold its own copies of every object it started
with, so that no maintenance of the source repository can remove objects a
live clone needs.

This replaces a requirement that maintenance not run while agents are live.
That one managed the hazard of a clone that *references* a mirror's objects,
where pruning the mirror removes objects a live clone needs and the failure
surfaces inside the agent long after, as corruption. A hardlinked clone does
not have that hazard (nor, since 2026-10-12, a copied one), so there is
nothing to schedule around.

#### Scenario: The source repository is pruned while agents run

- **WHEN** the source repository rewrites its history and prunes every
  unreachable object while agents are running
- **THEN** each agent's clone stays whole, every commit it had still present

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

#### Scenario: A manifest grant would reach the daemon or write the store

- **WHEN** an agent's policy grants a path containing the package-manager
  daemon's socket, or grants the host-global store read-write
- **THEN** the agent does not start, and the refusal names the path and the
  authority it would have conferred

#### Scenario: The host-root strategy on a host running the daemon

- **WHEN** fleet is asked for the host-root strategy on a host whose
  package-manager daemon socket exists
- **THEN** it refuses before creating anything, naming the socket
- **AND** it says the authority cannot be withheld under that strategy at
  all (Landlock does not mediate connecting to a unix socket), rather than
  that it is not yet supported

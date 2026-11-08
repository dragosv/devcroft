## Context

`up` spawns the keeper with `Command` and a `pre_exec` that enters a user,
mount and (sometimes) network namespace and builds the view
(`confine_on_exec`). Fleet spawns an agent with `clone3`, whose child becomes
PID 1 (`fleet::init::helper_main`), builds the view with a fresh procfs,
applies the plan, and runs the agent's command (the keeper) as its child,
reaping orphans and forwarding termination signals.

## Decisions

### D1. The keeper cannot be the process that unshares the PID namespace

Measured: after `unshare(CLONE_NEWPID)`, `pthread_create` fails `EINVAL`
(`copy_process` refuses `CLONE_THREAD` when `pid_ns_for_children` differs
from the caller's own). The keeper has threads (the egress relay, russh), so
it must itself be *inside* the namespace. And a process inside must be PID 1,
which ignores signals it has no handler for and inherits every orphan, or sit
under one. So the shape is fleet's: PID 1 as an init, the keeper its child.

Rejected: a `pre_exec` that forks a reaper PID 1 and lets the outer process
exec the keeper. The keeper would be outside the namespace, which D1 forbids.

### D2. Reuse `fleet::init`, not a second init

Fleet's helper already does everything PID 1 needs, measured by its own
tests: the identity-map handshake before exec, a fresh procfs, the view,
`SECBIT_NOROOT` for the command, signal forwarding, reaping. `up` differs in
three things, which become options rather than a fork of the code:

- **No cgroup leaf.** `up` has no fleet node; `CLONE_INTO_CGROUP` is set only
  when a leaf is given.
- **Network namespace only when isolated.** A `network.default = "allow"`
  sandbox shares the host's loopback today and must keep doing so.
- **Hostname.** An `up` sandbox has always seen the host's; it is passed
  through rather than replaced by the sandbox name.

### D3. Decided before the plan, by probing

The `/proc` grant is part of the compiled plan, so whether the sandbox gets
its own procfs must be known before the plan exists. A re-exec'd
`__pidns_probe` does the real thing (user+mount+PID namespace, fresh procfs
mount) in a throwaway process, as `__mount_probe` and `__netns_probe` do.
Falling back on a failed spawn instead was rejected: the plan would already
carry a `/proc` grant that the fallback must not have.

### D4. `/proc` is granted only with a procfs the sandbox owns

The grant is the whole of `/proc` (a rule on `/proc/self` binds the granting
process's pid directory), so it is only narrow where procfs lists the
sandbox's processes alone. One origin for both callers, `pid-namespace`,
because the condition is the namespace, not the command.

## Risks

- **Keeper pid.** `keeper.pid` now names PID 1's host pid. `down` signals it,
  PID 1 forwards SIGTERM to the keeper, and PID 1's exit ends the namespace,
  so `down`'s SIGKILL escalation still ends everything.
- **Pids are the sandbox's numbering.** `$$` in a session, a service's pid in
  `status`, and what the keeper reports are numbers in the sandbox's own
  namespace, not the host's. Found by three tests that took such a number and
  checked or signalled it on the host; one sent `SIGKILL` to it, which on the
  host names some other process. They now translate it
  (`tests/common/host_pid.rs`, by `NSpid` within the keeper's namespace). A
  host-side tool that does the same needs the same translation.
- **Hosts that mask `/proc`** (Docker without `systempaths=unconfined`) keep
  today's behaviour; the warning names the remedy fleet already prints.

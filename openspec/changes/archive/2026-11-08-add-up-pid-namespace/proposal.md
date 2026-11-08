## Why

An `up` sandbox shares the host's PID namespace and bind-mounts the host's
procfs, so `/proc` stays ungranted: granting it would let a sandbox read
`/proc/<pid>/environ` of every process its user owns. That default deny is
what `docs/known-gaps.md`'s "No inter-sandbox process visibility separation"
relies on, and it is also what stops a real agent. Measured 2026-10-18
(`add-agent-workload` 6.3): Claude Code is a Bun binary that aborts in 3 ms
without `/proc/self/{maps,cgroup,stat}` and `/proc/stat`. Fleet agents
already get a PID namespace and a fresh procfs; `up` sandboxes do not.

## What Changes

- On a Linux host that can mount a fresh procfs in a user+PID namespace,
  `up` starts the keeper (and the provisioning process) as the child of a PID
  1 of its own, in a new PID namespace, with a fresh `/proc` in the view.
  This is fleet's init helper (`fleet::init::spawn`), generalized: no cgroup
  leaf required, the network namespace only when the sandbox already gets
  one, and the host's hostname kept.
- In that case only, the compiled policy grants `/proc` read-only, origin
  `pid-namespace`, so runtimes that read their own `/proc` work, and a
  sandbox sees its own processes and no one else's.
- A host that cannot (Docker's masked `/proc`, no user namespaces) keeps
  today's behaviour with exactly one warning naming what is lost; `/proc`
  stays ungranted there.
- Fleet's `fleet:proc` origin becomes `pid-namespace`: the rule and its
  reason are the same in both.

## Capabilities

### Modified Capabilities

- `lifecycle`: the keeper runs in its own PID namespace where the host allows.
- `policy`: `/proc` is granted only on a procfs the sandbox owns.

## Impact

- `src/fleet/init.rs` (leaf and network optional), `src/lifecycle/up.rs`
  (spawn through it, probe, grant), `src/policy/mod.rs` (origin),
  `src/bin/devcroft.rs` (probe subcommand), `docs/known-gaps.md`.
- Behaviour: `ps` inside a sandbox lists only its processes; the keeper's
  pidfile names PID 1's host pid, which forwards `down`'s SIGTERM.

## 1. Init helper, generalized

- [x] 1.1 `fleet::init::spawn` takes an optional leaf; `CLONE_INTO_CGROUP`
      only with one.
- [x] 1.2 `AgentSpec` can share the host's network namespace; the helper then
      skips bringing loopback up.
- [x] 1.3 Fleet's own tests pass unchanged.
      Those that run here do (`fleet_project`, `fleet_init`'s non-cgroup
      ones). **Those needing a delegated cgroup were skipped**, as before this
      change: `sudo` wants a password in this devcontainer.

## 2. `up`

- [x] 2.1 `__pidns_probe`, and `up` decides with it before the plan.
- [x] 2.2 The keeper starts through the init helper when the probe passes,
      with its listeners inherited and the host's hostname.
      `spawn_through_init`: the command `Command` already describes, with its
      args rewritten to the helper's inherited range, and the relay bound by
      the helper (the keeper starts restricted and could not bind it).
- [x] 2.3 The provisioning process too.
- [x] 2.4 `/proc` granted, origin `pid-namespace`, only then; recorded in
      `Meta` so `policy --render` shows it.
- [x] 2.5 One warning when the probe fails, naming what is lost and the
      remedy.

## 3. Tests

- [x] 3.1 A session lists only its sandbox's processes in `/proc`, and reads
      its own `/proc/self/status`.
- [x] 3.2 A process outside the sandbox is not visible in its `/proc`.
- [x] 3.3 `down` still ends every process in the sandbox.
      `tests/up_pid_namespace.rs`: a process left running detached is gone
      after `down`, and existed before it; with the PID namespace disabled the
      test fails. Measured on the way: the keeper takes ~1.9 s to exit on
      SIGTERM (it waits `SESSION_GRACE` for its sessions), so `down` usually
      reaches its SIGKILL of PID 1, which now ends the whole namespace.
      Real agent: Claude Code 2.1.302 runs in an `up` sandbox and `claude -p`
      reaches the API through the proxy.
- [x] 3.4 The full suite, including `tests/process_tier_landlock_boundaries.rs`.
      697 passed, 0 failed (2026-10-18), no keeper left running after it.
      The Landlock boundary test now expects each regime's own answer: with a
      PID namespace the victim does not exist inside (`No such process`,
      `No such file or directory`); sharing the host's, V6 scoping refuses it.
      So on a host with PID namespaces, V6 signal scoping is no longer what
      that test exercises.

## 4. Docs

- [x] 4.1 `docs/known-gaps.md`: process visibility fixed where the host
      allows; the Bun section updated.
- [x] 4.2 CLAUDE.md's origin list.

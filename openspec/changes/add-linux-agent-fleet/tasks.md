# Tasks — Linux Agent Fleet

Ordered by what blocks what. Phase 0 gates the design; do not write the rest of
the implementation before it resolves.

## 0. Blocking verification

- [x] Ruleset construction is separable from application — `PreparedLandlockSandbox`,
      `prepare_seccomp_with_abi`, `RawSandboxError` (D1). No upstream work needed.
- [x] Determine whether `resource::ResourceLimits` is rlimits or cgroup-backed.
      **Answered, and the binary framing does not survive it.** It is
      cgroup-backed *in intent* — the fields document themselves as `memory.max`
      + `memory.swap.max=0` + `memory.oom.group=1` (`memory_bytes`) and
      `pids.max` (`max_processes`). But **section 1 does not collapse**, because
      the type is a declaration and nothing more: the whole public surface is
      `is_empty()`, `summary()`, `parse_size()` and `format_bytes()`. The
      module's own doc says the rendering to cgroup v2 lives in **`nono-cli`'s
      `resource_cgroup`** — the binary `use-nono-library` removed devcroft's
      dependency on. Nothing in the crate writes a cgroup file, and
      `cgroup.kill`, `cpu.stat`, `memory.current`, `cgroup.events`, `cpu.weight`
      and `io.weight` appear nowhere in it.
      So the library supplies two of D6's four limits as config, plus size
      parsing and formatting. Delegated scope creation, applying the limits,
      atomic teardown, metrics and the preflight check are all devcroft's, as
      section 1 already lists them. Sizing unchanged; one struct reused
- [ ] Decide whether the re-exec helper is needed, or whether the library's
      child-safe operations cover everything the supervisor does after clone (D2).
- [ ] Pin the crate to an exact version and record the upgrade policy (D10).
- [ ] Confirm the snapshot layer is the content-addressable `undo` module rather
      than an overlay, and measure agent startup cost at N agents on a
      representative worktree (Open Question 3).
- [ ] Decide the supported kernel floor and the degradation behaviour below it
      (Open Question 6). Note `SeccompNetFallback` and
      `probe_seccomp_block_network_support` already provide a network-blocking
      fallback below the Landlock network ABI.

## 1. Resource control

- [ ] Implement delegated slice creation and per-agent scope creation.
- [ ] Apply memory, CPU weight, IO weight and PID limits from configuration.
- [ ] Implement teardown via `cgroup.kill`.
- [ ] Read metrics and exit events from the agent's cgroup interface files.
- [ ] Preflight check for cgroup v2 delegation with an actionable diagnostic.
- [ ] Test: runaway build in one agent leaves other agents schedulable.
- [ ] Test: stopping an agent with orphaned descendants leaves nothing alive.

## 2. Sandbox runtime

- [ ] Implement the internal `devcroft-init` subcommand: single-threaded, config
      over pipe, ruleset over inherited fd.
- [ ] Implement namespace creation (net, pid, ipc, uts, mount).
- [ ] Implement the mount plan: read-only system layer with merged-`/usr`
      symlinks, private `/proc`, minimal `/dev`, private `/tmp`, workspace bind.
- [ ] Verify the agent command, its language runtime, its config directories and
      CA certificates are all present in the constructed view.
- [ ] Wire ruleset construction in the parent, namespace-local rule addition in
      the helper, application after mounts.
- [ ] Structured error reporting from the helper back to the supervisor.
- [ ] Test on at least two distributions, including one that restricts
      unprivileged user namespaces by default.
- [ ] Test: agent cannot see or signal another agent's processes.

## 3. Networking

- [ ] **Spike:** pasta vs slirp4netns — forwarding semantics, throughput, flag
      stability, packaging, teardown behaviour. Write the finding into `design.md`
      as the D5 resolution.
- [ ] Implement connectivity into each netns using the selected helper.
- [ ] Host one proxy instance per agent in the supervisor, outside the sandbox.
- [ ] Forward the proxy port into each agent namespace.
- [ ] Attribute requests to agents by listener; include the agent ID in audit
      logs.
- [ ] Test: no route out of the namespace except the forwarded proxy port.
- [ ] Test: agent B's request to a destination only agent A allows is refused.
- [ ] Revise any documentation claiming exfiltration is prevented.

## 4. Workspace isolation

- [ ] Implement the shared bare mirror and per-agent clone with `--reference`.
- [ ] Disable automatic GC on the mirror and all clones; add supervisor-driven
      maintenance when the fleet is idle.
- [ ] Remove or block the upstream remote in agent clones; implement the
      integration step for accepted sessions.
- [ ] Bind-mount the Nix daemon socket into each namespace.
- [ ] Implement per-agent GC roots, released on agent exit.
- [ ] Test: concurrent commits across agents, no spurious lock failures.
- [ ] Test: store GC during an active fleet retains all live paths.

## 5. Service ports

- [ ] Extend the environment schema with the declared port and optional host
      mapping.
- [ ] Allocate host ports per agent; release on exit.
- [ ] Report mappings in agent status.
- [ ] Wire the same schema into the macOS single-developer path.
- [ ] Test: five agents bind the same declared port; each host mapping reaches
      the correct agent.

## 6. Hygiene and follow-up

- [ ] Confirm the init helper leaves an insertion point between ruleset
      application and exec, for `add-syscall-filtering` (D9). No filter is
      implemented in this change.
- [ ] Integration test that exercises sandbox behaviour, to be run on every
      crate upgrade.
- [ ] Revisit the two-tier isolation model against the two-axis reality
      (Open Question 5); update the existing spec suite if the tiers collapse.
- [ ] Document the macOS-to-fleet path via a Linux VM, including where the
      worktree lives.

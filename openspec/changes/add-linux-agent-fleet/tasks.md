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
- [x] Decide whether the re-exec helper is needed, or whether the library's
      child-safe operations cover everything the supervisor does after clone (D2).
      **Decided: required.** Not because of the library — which does support
      raw clone — but because the child must enter namespaces, run the identity
      handshake, build a mount plan, become PID 1 and reap, and receive an
      inherited listener. The library removing one reason for re-exec does not
      remove the other five.
- [ ] Pin the crate to an exact version and record the upgrade policy (D10).
- [ ] Confirm the snapshot layer is the content-addressable `undo` module rather
      than an overlay, and measure agent startup cost at N agents on a
      representative worktree (Open Question 3).
- [ ] **Spike (blocking): the seccomp notification listener handoff.** The
      proxy-only filter traps `sendmsg`, so the listener FD cannot be passed
      over an ordinary control socket after installation. Validate a bootstrap
      (`CLONE_FILES`, or the pidfd route) that transfers it to the host's proxy
      loop. **No proxy work starts until this resolves** (D9).
- [ ] **Spike: slirp4netns with the exact flags fleet needs**, per supported
      distribution — `--disable-host-loopback`, explicit inbound forwarding, no
      automatic forwarding — verifying behaviour rather than binary presence
      (D5). Blocked in this devcontainer until `/dev/net/tun` is available.
- [ ] **Spike: systemd user-service delegation** — create the subtree, enable
      controllers, move a child into a leaf, `cgroup.kill` it, and observe
      `cgroup.events` report it empty (D6).
- [ ] Decide the supported kernel floor and the degradation behaviour below it
      (Open Question 6). Note `SeccompNetFallback` and
      `probe_seccomp_block_network_support` already provide a network-blocking
      fallback below the Landlock network ABI.

## 1. Resource control

- [ ] Run fleet as a systemd user service with `Delegate=yes`; discover the
      cgroup via `/proc/self/cgroup` rather than assuming a fixed path.
- [ ] Create the empty internal `fleet` node plus one domain leaf per agent;
      keep the supervisor and each agent's host-side proxy out of the leaves.
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
      **`net` is done** (`src/fleet/netns.rs`,
      `enter_network_namespace`) — the rest (pid, ipc, uts, mount) is
      not. Split out and built first because the D5 spike showed the
      network half is independently deliverable and is what
      `service-ports` rests on entirely.
- [x] **Bring `lo` up inside each agent's netns.** Found by the D5 spike:
      a fresh netns's loopback device is `DOWN` with no address, and a
      service bound there gets `bind()` success followed by client
      `ENETUNREACH` — it starts, reports healthy, and is silently
      unreachable, the exact failure `add-flox-services` exists to
      prevent. Belongs here rather than in group 3 (Networking): it needs
      no forwarding helper, no TUN device, and no privilege beyond the
      user namespace, and every in-namespace service depends on it.
- [x] Test: a service bound inside a constructed namespace is actually
      *reachable* from inside it, not merely bound. Asserting `bind()`
      succeeded would pass against the broken case above.
      `tests/fleet_netns.rs`. **Verified the tests actually fail when the
      feature is broken**, which caught a flaw in the tests themselves:
      the skip guard originally used the same probe as the assertion, so
      disabling `bring_loopback_up` made all four report `ok` — a
      regression was indistinguishable from an unsupported host. The
      guard now asks strictly less than the tests assert (namespace
      creation only), and with the feature disabled three of the four
      fail as they should.
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
      **Started; blocked on the devcontainer, findings recorded in
      design.md under D5.** Both candidates fail identically here with
      `open("/dev/net/tun"): No such file or directory` — this
      devcontainer has no `/dev/net` at all, so the comparison cannot be
      run. What the spike did settle: unprivileged user+net namespaces
      work, the `lo`-is-DOWN trap above, and that **service ports do not
      depend on this decision** (two agents were shown binding the same
      port with no helper and no TUN device). Egress is what waits on
      D5, not port isolation.
- [ ] **Prerequisite for resuming the spike:** pass `/dev/net/tun` into
      the devcontainer (`--device`), or run the comparison on a host that
      has it. Without this, neither candidate can be evaluated at all,
      and the selection criteria D5 names (throughput on loopback-heavy
      workloads especially) are unmeasurable.
- [ ] Implement connectivity into each netns using slirp4netns (D5's baseline),
      gated on the behavioural preflight above.
- [ ] Install the proxy-only seccomp filter and transfer its listener to the
      host proxy loop **before the keeper starts** (D9's phase-0 gate).
- [ ] Host one proxy instance per agent in the supervisor, outside the sandbox.
- [ ] Forward the proxy port into each agent namespace.
- [ ] Attribute requests to agents by listener; include the agent ID in audit
      logs.
- [ ] Test: a direct socket is refused by the seccomp policy **even though the
      network helper could route it**. The old wording ("no route out except
      the forwarded proxy port") tested the helper's configuration; the point
      is that the helper is not the boundary, so the test must defeat it.
- [ ] Test: agent B's request to a destination only agent A allows is refused.
- [ ] Revise any documentation claiming exfiltration is prevented.

## 4. Workspace isolation

- [ ] Implement the shared bare mirror and per-agent clone with `--reference`.
- [ ] Disable automatic GC on the mirror and all clones; add supervisor-driven
      maintenance when the fleet is idle.
- [ ] Remove or block the upstream remote in agent clones; implement the
      integration step for accepted sessions.
- [ ] Mount the provider's **resolved runtime paths read-only** into each
      agent (closure paths for closure-tier providers; devcroft-owned artifact
      paths plus explicit host library grants for qualified artifact-tier ones).
      **Replaces "bind-mount the Nix daemon socket" and "per-agent GC roots",
      which are struck.** Those tasks assumed agents hold package-manager
      authority; `sandbox-provisioning` P2a/P2b establishes they must not, and
      the multi-agent case is where that matters most — a host-global store is
      shared by every agent, so granting one agent's project code authority
      over it is authority over every other agent's toolchain. With no daemon
      socket in any agent there are no per-agent GC roots to manage either.
- [ ] Refuse, naming the requested authority, any workflow that needs a
      package-manager daemon or a writable host-global store.
- [ ] Test: concurrent commits across agents, no spurious lock failures.
- [ ] Test: store GC during an active fleet retains all live paths.

## 5. Service ports

> Now has a normative delta spec (`specs/service-ports/spec.md`), written
> before implementation because five of this change's six declared
> capabilities had none and tasks are not acceptance criteria. Writing it
> settled two things these tasks had left ambiguous — see 5.1.

- [ ] 5.1 Declare the port and optional host mapping in **devcroft's own
      manifest, keyed by service name**, sharing `add-port-allocation`'s
      configuration surface.
      **This replaces "extend the environment schema", which was not
      implementable as written.** devcroft reads services from the
      *provider's* manifest and models them as `provider::ServiceDecl`
      (name, command, per-service `vars`, daemon flags) — a mirror of
      flox's documented `[services]` schema, which devcroft consumes and
      does not own. There is no port field to extend, and adding one
      upstream is not devcroft's to do. The port lives in the command
      string or in `vars`, neither of which devcroft can reliably parse,
      so the declaration has to be devcroft's own.
- [ ] 5.2 Start each agent's declared service stack under that agent's keeper
      and inside its cgroup leaf; gate agent readiness on those services being
      ready, so a task dispatched to a ready agent does not race its own
      database coming up.
- [ ] 5.3 Allocate host ports per agent; release on exit.
- [ ] 5.4 Report mappings in agent status, distinguishing "no mappings
      declared" from "mappings not yet established".
- [ ] 5.5 Wire the same schema into the macOS single-developer path, and
      surface the degradation there rather than letting a shared port
      read as a private one.
- [ ] 5.6 Test: five agents bind the same declared port; each host mapping
      reaches the correct agent.
- [ ] 5.7 Test: a service whose command hardcodes its port runs unchanged
      in every agent, with no warning. **The second half is the
      assertion that matters** — the same manifest under
      `add-port-allocation` must fail loudly, and a test that only checks
      "it works" would pass equally against an implementation that had
      wrongly copied that change's refusal into fleet.
- [ ] 5.8 Test: a declared port naming a service the provider does not
      declare fails at `up`, distinguishably from a service that failed
      to start.

## 6. Hygiene and follow-up

- [ ] Confirm the init helper leaves an insertion point between applying the
      sandbox ruleset and starting the workload, for `add-syscall-filtering`.
      **Reworded: D9 reversed.** This used to add "No filter is implemented in
      this change", which is no longer true — the proxy-only seccomp filter is
      now mandatory where egress is granted. What stays deferred is *general*
      syscall-surface hardening, which is what the seam is for.
- [ ] Integration test that exercises sandbox behaviour, to be run on every
      crate upgrade.
- [x] ~~Revisit the two-tier isolation model against the two-axis reality.~~
      **Struck:** `remove-gvisor-backend` deleted the second tier, so the axis
      collapsed on its own. The remaining axis is single-environment versus
      fleet, which is this change's subject.
- [ ] Document the macOS-to-fleet path via a Linux VM, including where the
      worktree lives.

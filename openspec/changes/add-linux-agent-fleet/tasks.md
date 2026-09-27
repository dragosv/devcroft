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
- [x] **Re-derive whether D9's filter is needed at all — do this before the
      spike below, not after.** D9 declared the proxy-only seccomp filter
      mandatory and made the handoff spike a hard gate, reasoning that a
      userspace network helper providing a general stack makes proxy
      environment variables cooperative. **The shipped design has no such
      helper**: the namespace has loopback only and egress is a relay to a
      unix socket (`add-egress-proxy` E7), so a workload ignoring
      `HTTPS_PROXY` is refused by Landlock's `NetPort` *and* by there being
      no route out. If that holds for a fleet agent, both items below stop
      being blockers and this group loses its hardest work.
      **Answered (2026-09-26, measured): not needed — conditionally, and
      the "refused twice over" framing was wrong.** The two layers are not
      redundant; they cover different axes. A Landlock-restricted child
      (ABI 8, `ConnectTcp` for one port P only):

      | probe | host netns | fresh userns+netns, `lo` only |
      | --- | --- | --- |
      | `127.0.0.1:P` | connected | connected |
      | *non-loopback address*`:P` | **connected** | connected (`127.0.0.2`) |
      | `127.0.0.1:Q` (not granted) | EACCES | EACCES |
      | `1.1.1.1:443` | EACCES | EACCES |
      | UDP `sendto 1.1.1.1:53` | **sent** | ENETUNREACH |

      `NetPort` scopes by **port number alone, never by address**, and
      is TCP-only. So Landlock alone would let a workload reach *any
      host* on the proxy's port (or a declared `network.ports` port), and
      any host at all over UDP. What makes egress non-cooperative is the
      namespace having no route out; Landlock is the TCP-port second
      layer inside it. The filter is therefore unnecessary **exactly as
      long as an agent's namespace never gains a route** — which is the
      invariant D5 would break (see the slirp4netns item below).
      Probe: a ctypes script calling `landlock_create_ruleset`/
      `add_rule(NET_PORT)`/`restrict_self` directly, so the result is the
      kernel's, not nono's.
- [ ] ~~**Spike (blocking): the seccomp notification listener handoff.**~~
      **Gate lifted by the re-derivation above, conditionally**: not
      needed while agent namespaces stay route-less. It comes back the
      moment anything gives an agent a general stack — and then it is
      needed for UDP as much as for TCP, since `NetPort` covers neither
      addresses nor datagrams. The spike itself is still the right work
      *if* the filter is needed. The
      proxy-only filter traps `sendmsg`, so the listener FD cannot be passed
      over an ordinary control socket after installation; a bootstrap
      (`CLONE_FILES`, or the pidfd route) would have to transfer it to the
      host's proxy loop. What is no longer true is "**no proxy work starts
      until this resolves**" — proxy work shipped, without the filter, and
      the egress it produces is non-cooperative by construction (D9).
      **And if it is needed, a working sequence already exists to copy.**
      `sandlock` (`docs/prior-art.md`) does the handoff by ordering rather
      than by a trick: fork, child installs the filter and receives the
      listener FD, child transmits that FD to the parent *as part of
      installation* — before anything is trapped — then blocks on a
      "ready" signal until the parent's supervisor is live on it, and only
      then execs. The chicken-and-egg this task describes exists only if
      the FD is passed *after* the filter is enforcing, which suggests the
      difficulty is specific to nono's choice to trap `sendmsg` rather
      than inherent. Read that sequence before spiking.
- [ ] **Spike: slirp4netns with the exact flags fleet needs**, per supported
      distribution — `--disable-host-loopback`, explicit inbound forwarding, no
      automatic forwarding — verifying behaviour rather than binary presence
      (D5). Blocked in this devcontainer until `/dev/net/tun` is available.
      **Scope reduced to the host-side port mapping**: reaching the proxy and
      (through it) a registry needs no helper, since a unix socket crosses a
      network namespace. Nothing in fleet's in-namespace half waits on this.
      **Re-examine before spiking — the D9 re-derivation makes this the
      wrong tool.** slirp4netns attaches a tap with a default route and has
      no flag that disables outbound (`--disable-host-loopback` only covers
      the host's loopback). Attaching it for *inbound* mapping therefore
      hands the agent a route out, and with it UDP to anywhere and TCP to
      any host on a granted port — exactly D9's premise, which reinstates
      the filter and its handoff gate. The alternative keeps the invariant:
      E7's stream relay in the other direction (host TCP listener → a
      pathname unix socket → the in-namespace forwarder, bound before
      restriction → `127.0.0.1:<port>`). That needs no `/dev/net/tun`,
      no helper binary and no helper preflight. Unverified: whether the
      forwarder's Landlock profile grants `ConnectTcp` to its own declared
      ports (`allow_localhost_port` suggests yes).
- [ ] **Spike: systemd user-service delegation** — create the subtree, enable
      controllers, move a child into a leaf, `cgroup.kill` it, and observe
      `cgroup.events` report it empty (D6).
      **Cannot run in this devcontainer, measured (2026-09-26).** PID 1 is
      `sh`, there is no systemd and no `/run/systemd`. The unified
      hierarchy is mounted `ro,nsdelegate`, owned by root, with an empty
      `cgroup.subtree_control` (controllers `cpuset cpu io memory pids`
      are available, none delegated); the container sits at `0::/` in its
      own cgroup namespace. The capability bounding set is Docker's default
      (`0xa80425fb`, no `CAP_SYS_ADMIN`), so root could not remount it
      either. The last route fails too: a fresh userns+mountns+cgroupns
      *can* mount cgroup2, but every file shows as `65534:65534`, and both
      `mkdir` and writing `cgroup.subtree_control` get EACCES.
      Making it work here would take `--privileged` or systemd-as-init.
      That is a far larger standing relaxation than `/dev/net/tun`, and it
      still would not test D6's actual target, a systemd **user** manager
      with `Delegate=yes`. Run this spike on a real systemd host or a
      Linux VM with systemd as PID 1.
      **Partly unblocked since then:** the devcontainer now delegates
      `/sys/fs/cgroup/delegated` to `vscode` (`--cap-add=SYS_ADMIN` plus
      `.devcontainer/cgroup-delegate.sh`, dind-style nesting). Run
      `sudo cgroup-delegate enter $$` first. That covers the cgroup
      mechanics: subtree, controllers, leaf, `cgroup.kill`, `cgroup.events`.
      It does not cover the systemd half (user manager, `Delegate=yes`,
      discovering the `user@<uid>.service` ancestor), which still needs the
      VM. Measured after the rebuild: the root is empty, all five
      controllers are available in `delegated/`, and after `enter` the dev
      user can `mkdir` a child and enable `+memory` there without root.
      **Cgroup half run (2026-09-26); design.md D6 "Measured" has the
      numbers.** Top-down enablement, a domain leaf, `clone3` straight into
      it, a group OOM kill, `pids.max`, and `cgroup.kill` of 20 orphans
      (`populated 0` in 1.9 ms) all behave as D6 assumes. Four results
      change section 1: `io.weight` can be missing while `io` is enabled;
      `CLONE_INTO_CGROUP` removes the attach race; without
      `memory.swap.max=0`, swap turns `memory.max` into no cap at all; and
      agents own their cgroup files, so a leaked cgroup fd lets one agent
      rewrite another's limits. Left open: the systemd half, on a VM.
- [ ] Decide the supported kernel floor and the degradation behaviour below it
      (Open Question 6). Note `SeccompNetFallback` and
      `probe_seccomp_block_network_support` already provide a network-blocking
      fallback below the Landlock network ABI.

## 1. Resource control

> **A working reference exists and is license-compatible.** `nono-cli`'s
> `crates/nono-cli/src/resource_cgroup.rs` implements this group's cgroup
> half — task 0 already noted that file as where the rendering lives, and
> nobody had read it. It is Apache-2.0, and devcroft is now Apache-2.0
> too, so it can be *adapted with attribution* rather than only studied.
> See `docs/prior-art.md` for what it contains and the four details below
> that this task list did not have.
>
> It also **independently reaches D6's conclusion**: delegation
> unavailable means the run is refused at setup, with no fallback — the
> same "a hard preflight failure beats a fallback that silently
> under-enforces" this change decided on its own.

- [ ] Run fleet as a systemd user service with `Delegate=yes`; discover the
      cgroup via `/proc/self/cgroup` rather than assuming a fixed path.
      Parse for the `0::` unified-hierarchy prefix, walk to the
      `user@<uid>.service` ancestor, reject v1/hybrid, and **match path
      segments exactly** — the reference does the last one specifically to
      stop path traversal, which is not obvious from "parse
      /proc/self/cgroup".
- [x] **Verify requested controllers appear in the parent's
      `cgroup.subtree_control` and fail if not.** This is D6's
      "silently under-enforce" concern as a concrete check: without it a
      missing controller means the limit is configured and absent.
      **Check the interface file too, not only the controller**: measured,
      `io` enables cleanly on a host whose leaves have no `io.weight`
      (the `none` scheduler; weights need BFQ or iocost). A missing
      `io.weight` is D6's named degraded capability. On a host with swap
      on, a missing `memory.swap.max` is a refusal: the next task shows
      `memory.max` alone does not cap anything there.
- [ ] **The child starts inside its leaf: `clone3(CLONE_INTO_CGROUP)` in
      D2's clone, together with `CLONE_NEWCGROUP`.** Otherwise there is a
      window where the child runs uncapped in the parent's cgroup. The
      reference closes it by having the child write its pid to an inherited
      `cgroup.procs` fd right after `fork`. Measured, `CLONE_INTO_CGROUP`
      closes it outright: the child's first instruction already sees the
      leaf. In the same call it roots the child's cgroup namespace at the
      leaf, which is what refuses an agent raising its own limits
      (`nsdelegate`) or moving itself out. The fd-write route also works
      and is the fallback, but the 5.14 `cgroup.kill` needs already
      includes clone3 (5.7).
      **The fallback is built** (`Leaf::attach_on_spawn`, a `pre_exec`
      write); the `clone3` route lands with D2's init helper.
- [ ] **Every cgroup fd the supervisor opens is `O_CLOEXEC`**, including
      the leaf fd handed to `clone3`. Agents run as the uid that owns
      every delegated file, so nothing but reachability protects a
      sibling's limits. Measured: an agent in its own cgroupns, with `/sys`
      hidden, set a sibling's `memory.max` to 1M through
      `openat(leaked_fd, "../b/memory.max")`. Test it from the agent side:
      `/proc/self/fd` holds no cgroupfs fd, and a sibling's interface files
      are unreachable by path.
      **Done for `src/fleet/cgroup.rs`**: `Leaf::open_dir` is `O_CLOEXEC`,
      asserted by test, and `attach_on_spawn`'s fd is std's (also
      close-on-exec). The agent-side test waits for the init helper.
- [x] **Leave `memory.high` unset when swap is off.** With
      `memory.swap.max=0`, a program over `memory.high` stalls instead of
      being killed, which looks like a hang rather than a limit. Set
      `memory.max` plus `memory.oom.group=1` so the whole leaf dies
      together. **`memory.swap.max=0` is load-bearing, not tidying**:
      measured on a host with zram swap, `memory.max=128M` with swap left at
      `max` let a 512 MB allocation succeed (384 MB swapped, nothing
      killed). With `swap.max=0` it was killed in 10 ms, the whole leaf with
      it, and `memory.events` recorded `oom_group_kill 1`.
- [x] Create the empty internal `fleet` node plus one domain leaf per agent;
      keep the supervisor and each agent's host-side proxy out of the leaves.
      Keeping the supervisor and proxy out is the caller's placement; the
      module only makes leaves.
- [x] Apply memory, CPU weight, IO weight and PID limits from configuration.
      **As `fleet up` flags** (`--memory 4G`, `--pids N`, `--cpu-weight W`,
      `--io-weight W`), validated before anything starts: weights 1-10000,
      sizes in nono's syntax (`nono::resource::parse_size`), exit 2
      otherwise. They reach every agent's leaf and its record; `inspect`
      shows them. An IO weight this host cannot apply is reported by name
      with its fallback, and only when one was asked for (the spec's
      *IO controller is unavailable*). Flags rather than a `[fleet]`
      manifest section so the config schema is untouched; a section can
      come later if limits should be committed with the project. A mutant
      that drops the limits fails.
- [ ] **Wall-clock timeout, which needs no cgroups and is missing entirely.**
      devcroft has no execution limit of any kind today — a runaway agent
      runs until someone notices. A timer plus the escalating
      SIGTERM/SIGKILL `state::terminate_and_wait` already implements is
      the whole mechanism, and unlike memory or CPU it does not depend on
      D6's systemd delegation. Worth landing ahead of the cgroup work
      rather than with it: it is the cheapest bound on an unattended
      agent, and `timeout 30 devcroft exec …` only helps a human who is
      watching. Noted from `sandlock`, which exposes it as `-t/--timeout`
      (`docs/prior-art.md`).
- [x] Implement teardown via `cgroup.kill` (Linux 5.14+), then poll
      `cgroup.procs` until the kernel has reaped — the reference polls 50
      times at 10ms. Measured: 20 orphaned, SIGTERM-ignoring daemons were
      reaped before the first 1 ms poll, so that budget is generous. Leave the directory behind with a warning rather than
      blocking forever if it does not drain.
- [ ] **Sweep stale leaves from crashed supervisors** before creating a
      new one, confirming the owning pid is gone via `/proc/<pid>` first.
      devcroft already applies exactly this reasoning to pidfiles
      (`state::is_same_process`), so it is the same rule in a second
      place, not a new one.
      **Done differently, by `Supervisor::reconcile`:** a leaf's own
      `populated` is the liveness test, not a recorded pid, since nothing
      but the supervisor creates leaves under its node and membership
      survives reparenting and restarts. A Running record whose leaf is
      empty is retired and its leaf removed; a directory with no record (a
      start interrupted by a crash) is removed with its leaf. Unticked
      until leaves with no directory at all are swept too.
- [x] Read metrics and exit events from the agent's cgroup interface files.
      **Done:** `fleet ls` and `inspect` read `memory.current` and
      `cpu.stat`, and the evidence is kept in the agent's record when it
      dies. It is read *before* the leaf is removed, since the counters go
      with it, both on `stop` and when reconciling an agent that died on
      its own. An agent OOM-killed at 64 MiB is retired with
      `oom_group_kill` recorded, and a clean stop records nothing; a mutant
      that drops the evidence fails.
      **Including why something died**: `memory.events`' `oom_kill` counter
      and `pids.events`' denied-fork count turn "the agent vanished" into
      "the agent was OOM-killed". Report nothing when the counters are
      zero, so a clean run stays quiet.
      **Exit evidence done** (`Leaf::exit_evidence`, `Evidence::is_quiet`);
      usage metrics (`memory.current`, `cpu.stat`) wait for `ps` to consume
      them.
- [ ] Preflight check for cgroup v2 delegation with an actionable diagnostic.
      **The cgroup half is done** (`FleetNode::create`): cgroup2 filesystem,
      required controllers present, each enable read back, swap without
      `memory.swap.max` refused, and `EBUSY` explained as the
      no-internal-process rule. Finding the delegated root is the systemd
      half, still open.
- [ ] Test: runaway build in one agent leaves other agents schedulable.
- [x] Test: stopping an agent with orphaned descendants leaves nothing alive.
      `tests/fleet_cgroup.rs`: five `setsid` daemons ignoring SIGTERM,
      reparented away from the test, all gone after `Leaf::kill`.

## 2. Sandbox runtime

- [x] Implement the internal `devcroft-init` subcommand: single-threaded, config
      over pipe, ruleset over inherited fd.
      **Built, with the ruleset applied in the helper rather than carried
      over an fd** (see the ruleset task below for why). **The keeper runs
      as the agent's command** (`__keeper 5 6`): the supervisor binds the
      control and SSH sockets on the host, `Stdio::inherit` hands them to
      the command at `FIRST_INHERITED_FD` onward, and PID 1 closes its own
      copies. `the_keeper_runs_as_the_agents_command_and_serves_sessions`
      runs a session over the ordinary keeper protocol: it lands in the
      agent's leaf, hostname, PID namespace and view, writes to the
      project, and dies with the agent.
      **First version, without the ruleset** (`src/fleet/init.rs`, `__fleet_init`):
      one `clone3` into all seven namespaces, directly into the cgroup leaf,
      with a pidfd. The child does only `dup2` and `execve`. The parent
      writes the identity map, then the configuration. The helper is PID 1:
      it mounts a fresh `/proc`, sets the hostname, brings `lo` up, starts
      the command under `SECBIT_NOROOT`, reaps, forwards SIGTERM/INT/HUP/
      QUIT, and exits with the command's status. The mount view, Landlock
      and the keeper slot in before the command starts.
      **A fresh `/proc` is refused under Docker's default masking**
      (measured: `unshare --mount-proc` gets EPERM), and without one a PID
      namespace hides nothing. The devcontainer now runs with
      `systempaths=unconfined`; `tests/fleet_init.rs` skips on a masked host
      and says why. On this host before the rebuild, the handshake ran end
      to end and reported `mount a fresh /proc: EPERM` by name in 5.7 ms,
      leaving the leaf empty.
      **After the rebuild, the first run found a race D2 had already
      warned about:** the child exec'd before the parent wrote `uid_map`,
      and an `execve` by an unmapped (non-zero) uid drops every capability.
      About half the agents failed at their first mount with EPERM. The
      child now blocks in `read` on a sync pipe until the map exists. 20 of
      20 runs are green since. Mutants checked: without `SECBIT_NOROOT` the
      `CapEff` assertion fails; without the fresh `/proc`, 47 processes are
      visible and agent a sees the host's `bash`.
- [x] Implement namespace creation (net, pid, ipc, uts, mount).
      **All of them, plus cgroup, in `fleet::init::spawn`**, asserted from
      inside the agent by `tests/fleet_init.rs`.
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
      **Consume `add-mount-isolation` rather than implementing this
      twice.** That change splits the same work out for the single-sandbox
      case, because a measured gap needs it now: Landlock does not mediate
      AF_UNIX, so every sandbox today reaches any world-accessible unix
      socket, and a mount view is what closes it. Same relationship fleet
      already has to `fleet::netns` — the primitive ships for one sandbox
      first, fleet is the second consumer.
      **Consumed** (`AgentSpec::view`): the helper resolves the plan's
      grants before `pivot_root`, with the same resolver Landlock uses, then
      calls `construct_view` with `ProcMount::Fresh`. That is the one
      change the fleet case needed, since `up` binds the host's `/proc`
      (no PID namespace there). `None` keeps the host root, which is D2a's
      other strategy, so the choice is the caller's to record.
      `with_a_view_an_agent_sees_exactly_its_grants`: the root holds only
      `bin dev etc lib proc sbin tmp usr`; the repository and `/etc/passwd`
      are absent; `/proc` is the fresh one; a file written in the project
      root is the host's and owned by the real user; and the view directory
      stays empty on the host. Mutant: with `HostBind`, 37 processes are
      visible. Leave unticked for the `/workspace` fixed path (D2a), which
      is workspace isolation's (group 4).
- [ ] Verify the agent command, its language runtime, its config directories and
      CA certificates are all present in the constructed view.
      **The runtime half, for one provider:** a real devbox closure builds
      and runs the sample inside an agent's view (`tests/fleet_commands.rs`).
      Config directories and CA certificates are not checked yet.
- [x] Wire ruleset construction in the parent, namespace-local rule addition in
      the helper, application after mounts.
      **Built differently, after measuring:** the helper builds *and*
      applies the ruleset after the view, from the plan in its
      configuration (`to_capability_set` plus `apply_auto`, as `up`'s
      keeper does), so PID 1 is confined too (`NoNewPrivs: 1` on PID 1,
      asserted). As a mutant, preparing it before the view
      (`prepare_landlock_with_abi`, which opens the rule fds) fails two
      tests: the view's private `/tmp` is refused, and a `/proc` grant
      lands on the host's procfs. Moving only `to_capability_set` earlier
      proves nothing, since nono opens the fds inside `apply`.
      `on_the_host_root_the_plan_still_refuses_what_it_does_not_grant`
      covers D2a's other strategy: the repository is visible and refused.
      **What was reasoned before measuring:** D2 builds the ruleset in
      the parent because allocating after a `fork` is unsafe. The helper
      is a freshly exec'd, single-threaded process, though, so that reason
      is gone there. `up`'s keeper already restricts itself *inside* the
      view, from the same `CapabilityPlan`. Rules bind to inodes, so
      building them after `pivot_root` names what the agent actually sees.
      **nono adds no `/proc` rules (measured 2026-09-26, nono 0.77.0).** For
      a real plan (`allow = ["."]` plus a `/usr` provider grant), the
      `CapabilitySet` holds 12 rules, 0 of them under `/proc`, and
      `apply_with_abi_inner` adds a rule only for each entry in
      `fs_capabilities()`. Under that restriction every `/proc` read is
      refused (`/proc/self/{status,cmdline,maps}`, `/proc/1/cmdline`, and
      listing `/proc` and `/proc/self/fd`); only `readlink /proc/self/exe`
      works, because Landlock does not mediate `readlink`. So `/proc/self`
      is not the reason. **The reason to build rules after the view is
      `/tmp` and `/dev/pts`.** Both are baseline grants, and in the view
      each is a new filesystem instance (private tmpfs, private devpts) that
      does not exist in the parent. Rules bind to inodes, so rules built in
      the parent would attach to the host's `/tmp` and devpts, and the
      agent's own would be denied. `fleet::mount::setup_dev` already relies
      on the keeper restricting itself after entering the view. (Measured
      since; see the paragraph above.)
      **Found on the way, a bug in `up`:** a read-only grant on any
      `nosuid`/`nodev`/`noexec` mount failed `construct_view` with
      `EPERM`, because the remount dropped flags a user namespace locks.
      Fixed in `remount_readonly`, with `tests/mount_locked_flags.rs`
      (`docs/implementation-log.md`).
- [x] **Share the keeper's environment with `up`.** `lifecycle::KeeperEnv`
      is what `spawn_keeper` used to build inline (SSH key material, the
      resolved shell, hooks, services, `HOME`, the relay), moved with its
      comments; `spawn_keeper` and the fleet keeper test both call
      `vars()`. The test now passes real SSH keys, and the keeper's SSH
      server answers with an `SSH-2.0` banner on the socket the supervisor
      bound. `up`'s lifecycle, hooks, environment, services and SSH suites
      pass unchanged, with one skip (`rsync` not on `PATH`).
- [x] Structured error reporting from the helper back to the supervisor.
      One JSON line on fd 4 naming the failed step, or EOF plus the
      helper's exit status (`a_failing_step_is_reported_by_name`).
- [ ] Test on at least two distributions, including one that restricts
      unprivileged user namespaces by default.
- [x] Test: agent cannot see or signal another agent's processes.
      `agents_cannot_see_or_signal_each_other`: agent a's fresh `/proc`
      lists only its own helper, `sh` and `cat`, and `kill -0` on agent
      b's pid fails.

## 2b. Making N sandboxes affordable

> Taken from ArcBox's `VmDriver` port (`docs/prior-art.md`), which serves
> `Prepare` and `Checkpoint` alongside plain boot. **None of it needs a VM** —
> the ideas are about amortising a per-sandbox startup cost, and devcroft's is
> provider resolution at `up`, paid in full every time.
>
> This group is here because fleet is where the cost stops being an annoyance
> and starts being the constraint: at N ≥ 3, per-sandbox startup is what a user
> actually experiences, and no amount of isolation elsewhere compensates.
> Recorded as tasks rather than left in `prior-art.md`, where a technique with
> no task is a note, not a plan.

- [ ] 2b.1 Measure first, and be willing to close this group. What does an
      `up` actually cost, broken down — provider resolution, closure
      materialisation, keeper spawn, restriction? If resolution is not the
      dominant term, the two ideas below are solving the wrong problem and
      should be dropped rather than built.
- [ ] 2b.2 A warm keeper: spawn ahead of the request, so a sandbox that is
      *about* to be asked for is already past the expensive part. ArcBox's
      `Prepare` spawns a VMM that then receives its spec; devcroft's analogue
      spawns a keeper that then receives its plan — the fd-passing shape it
      already uses.
- [ ] 2b.3 Establish what must *not* be shared across a warm instance before
      building it: the compiled policy, the project root, the resolved
      environment and the proxy token are all per-sandbox, so anything
      pre-spawned must be genuinely blank. A warm pool that leaks one
      sandbox's grants into another is worse than a slow `up`.
- [ ] 2b.4 A reusable resolution: the closest devcroft has to a checkpoint is
      that a resolved environment for one project root is the same for the
      next sandbox on it. Establish whether that is cacheable given
      `env_fingerprint`, and what invalidates it — this is not
      pause/snapshot/resume, and calling it "checkpoint" would overclaim.
- [ ] 2b.5 Whatever lands, `status`/`doctor` say when an instance came from a
      warm path rather than a cold one. A performance mechanism that cannot be
      observed cannot be debugged, and a warm instance carrying stale state is
      exactly the bug this would introduce.

## 2c. The supervisor

- [x] `fleet::Supervisor` (library): agents by stable ID (`a<N>`, never
      reused), each with a 0700 directory holding its 0600 control and SSH
      sockets, an ephemeral host key and `record.json`. The record is the
      spec's durable identity (workspace, cgroup, state, policy
      fingerprint, view strategy, port mappings, attention) and holds no
      pid. Start is all or nothing; stop kills one agent's leaf and nothing
      else; a restarted supervisor adopts live agents and retires dead
      ones. `tests/fleet_supervisor.rs`, with three mutants: reconcile that
      trusts the record, a failed start that keeps its directory, and a
      stop that never kills. The last one used to hang the suite (a
      blocking `wait` on a helper that never dies); `stop` now reaps with a
      bound and names the helper that outlived its leaf.
      **Two test bugs found on the way.** `cgroup.kill` is asynchronous, so
      a test that reopened the supervisor right after it found the agent
      correctly still running (11 of 15 runs); it now waits for
      `populated 0`. And the failed-start test checked for leftovers only
      after `list`, whose reconcile removed them, so it passed with no
      cleanup at all.
- [x] `devcroft fleet up --agents N | ls | stop <id>`, with the delegated
      cgroup root given explicitly (`--cgroup-root` or
      `DEVCROFT_FLEET_CGROUP_ROOT`, recorded in `fleet.json` for `ls`
      and `stop`). No daemon: liveness is the leaf, so each command opens
      the supervisor, reconciles and acts. `fleet::project::prepare`
      resolves each agent's environment for its own clone, as `up` does
      for a sandbox; `fleet::commands` holds the logic the CLI is thin
      over. Fleet state lives in `_fleet` under the data dir, a name no
      sandbox can have, and `ps` skips it.
      **Measured with a real provider:** on `devbox-citytime-sample`, two
      agents came up in 12 s, and a real `cargo build` from the devbox
      closure succeeded inside one, with the original repository
      unreadable (`a_real_devbox_agent_builds_its_project_in_its_clone`).
      **Found while building it:** a project in a subdirectory of its
      repository (every sample here) would have put the agent at the
      clone's root. The workspace is now the same relative path inside
      the clone; a mutant putting it back at the root fails.
- [x] `fleet inspect <id>` and `fleet rm <id> | --all [--yes]`. `rm` refuses
      a running agent, deletes its record and its clone (only at the path
      `up` makes clones, whatever the record says), and `--all` stops what
      is running, then removes every clone, the cgroup node and the state.
      `--yes` is required non-interactively, as for `rm`.
- [x] Preflight before the first agent (spec: *Preflight environment
      validation*), **by running one**: a probe agent runs
      `devcroft --version` in a throwaway node, so delegation, user
      namespaces, a fresh procfs and Landlock are each tested for real, and
      the step the helper names gets its remedy (`commands::remedy`). It
      runs before anything is cloned: an undelegated cgroup root is refused
      with exit 4 and no `.devcroft/` made.
      **Found by the tests:** the probe's node was named by pid, so two
      `up`s in one process shared its leaf and failed each other with
      EEXIST; it is now unique per call.
- [ ] The systemd user unit (`Delegate=yes`) and finding the delegated
      root from `/proc/self/cgroup` (section 1), on a VM with systemd.
- [ ] Per-agent workspaces are group 4's clones; today `AgentLaunch`
      takes any directory. Port mappings (group 5) and `attention`
      (`add-agent-interaction`) have their fields and nothing that sets
      them.

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
      **Not needed for egress, which is built without it** (next items):
      the relay to the proxy's unix socket crosses the namespace, so no
      helper, no TUN device and no route. Adding slirp4netns would give
      agents a route out and reinstate D9's filter (see the D9
      re-derivation). What is left of this item is inbound host port
      mapping (`service-ports`), for which the reverse relay above is the
      better candidate.
- [ ] Install the proxy-only seccomp filter and transfer its listener to the
      host proxy loop **before the keeper starts** (D9's phase-0 gate).
- [x] Host one proxy instance per agent in the supervisor, outside the sandbox.
      `Supervisor::start_in` spawns `__egress_proxy` for an agent whose
      `network.allow` is non-empty, with its socket and log in the agent's
      directory. The proxy runs in its own `<id>-proxy` cgroup leaf, beside
      the agent's and never inside it (D6), and is killed with the agent.
      `network.default = "allow"` is refused: the route-less namespace is
      the boundary.
- [x] Forward the proxy port into each agent namespace. Through the keeper's
      relay, the same mechanism `up` uses for an isolated sandbox. The
      helper binds the relay **before** restricting, because a keeper that
      starts restricted cannot, and hands it over as
      `DEVCROFT_PROXY_RELAY_FD`.
- [x] Attribute requests to agents by listener; include the agent ID in audit
      logs. One proxy per agent, logging to that agent's `egress.log`, and
      every record starts `agent=<id>` (`DEVCROFT_EGRESS_LABEL`,
      `proxy::server::run_labelled`), so a line still attributes its
      request once it is read anywhere else. `up`'s proxy is unchanged.
- [ ] Test: a direct socket is refused by the seccomp policy **even though the
      network helper could route it**. The old wording ("no route out except
      the forwarded proxy port") tested the helper's configuration; the point
      is that the helper is not the boundary, so the test must defeat it.
- [x] Test: agent B's request to a destination only agent A allows is refused.
      `each_agent_reaches_only_its_own_allowlist_through_its_own_proxy`: A
      and B each reach their own host and are refused the other's. A request
      bypassing the proxy (`--noproxy '*'`) gets nothing, since the
      namespace has no route. Stopping A removes its proxy and leaves B's
      working.
- [x] Revise any documentation claiming exfiltration is prevented. One did:
      `docs/decisions.md` said the tier protects against "simple
      exfiltration", against `docs/threat-model.md`. It now says the tier
      narrows where data can go and does not prevent exfiltration, since
      anything sent to an allowed host leaves. The other mentions
      (threat-model, known-gaps, comparison, this change's design) already
      said so.

## 4. Workspace isolation

- [ ] Implement the shared bare mirror and per-agent clone with `--reference`.
      **A minimal form exists** (`fleet::commands`): a `git clone --local`
      per agent at `<project>/.devcroft/fleet/<id>`, hardlinked objects, no
      alternates, so no GC hazard yet. The mirror, GC control and remote
      handling are still this task's.
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
- [x] 5.2 Start each agent's declared service stack under that agent's keeper
      and inside its cgroup leaf; gate agent readiness on those services being
      ready, so a task dispatched to a ready agent does not race its own
      database coming up.
      `fleet::project::prepare` writes each agent's supervisor config in its
      own workspace, named by its ID (`services::write_config`, extracted
      from `up`), and the keeper starts it. `fleet up` returns once every
      agent's services are running *and* past their readiness probes
      (`ServiceState::is_ready`, from process-compose's `has_ready_probe`
      and `is_ready`), or have failed, or have used up two minutes. Each
      outcome is per agent. A failed service is named for its agent, which
      stays up, and the exit status is 1. `fleet inspect` shows each
      service's live state.
      **With a real provider:** `flox-services-sample`, two agents, services
      ready in 4 s. Each agent asking `127.0.0.1:8710` for `whoami` got its
      own answer. Tested with a probe that passes only two seconds in, which
      fails if `up` does not wait; a failing service is reported with its
      agent left serving sessions.
      **Found on the way:** clones under the project put a subdirectory
      project's path in each workspace twice, and the supervisor socket
      came to 123 bytes against the OS's 103. Clones now live at the
      repository root.
- [ ] 5.3 Allocate host ports per agent; release on exit.
- [ ] 5.4 Report mappings in agent status, distinguishing "no mappings
      declared" from "mappings not yet established".
- [ ] 5.5 Wire the same schema into the macOS single-developer path, and
      surface the degradation there rather than letting a shared port
      read as a private one.
- [ ] 5.6 Test: five agents bind the same declared port; each host mapping
      reaches the correct agent. **The in-namespace half is tested** (three
      agents, one port, three instances each answering with its own
      workspace's file); the host mapping half waits for 5.3.
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
- [ ] **Harden keeper rediscovery beyond the pidfile.** devcroft decides a
      keeper is alive from a pidfile plus a health probe; a pidfile alone
      cannot distinguish a reused pid from the real process. ArcBox's `Adopt`
      holds every candidate — the recorded pid *and* `/proc` matches — to an
      identity test (`--id`, API socket, jail root) before adopting it
      (`docs/prior-art.md`). devcroft has the materials for the same check:
      `Meta` records `project_root`, and the control socket path is
      per-sandbox. Not fleet-specific, but it matters most at N, where pid
      reuse stops being hypothetical.
- [ ] Test the above against the case that motivates it: a pidfile naming a
      live pid that is *not* this sandbox's keeper must read as dead, not as
      healthy. A check that cannot fail is the one being replaced.
- [ ] Document the macOS-to-fleet path via a Linux VM, including where the
      worktree lives.

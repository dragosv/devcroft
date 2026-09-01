# Tasks — Mount Isolation

Ordered so the question that can invalidate the mount plan is answered
before the plan is written.

## 0. Measure the view before building it

> The spike that motivated this change masked *all* of `/nix`, which
> closed the daemon socket and would also have removed the toolchain. That
> is the whole difficulty of this change in one line: the correct view is
> narrow enough to close the gap and wide enough to compile. Guessing
> produces a sandbox that starts and fails at the first build.

- [x] 0.1 For each closure-tier provider (flox, nix, devbox), record what
      a real build actually opens: run one under `strace -f -e trace=file`
      or equivalent and diff against the provider's resolved paths. The
      question is specifically whether anything outside the resolved
      closure is needed. **Done, live: `flox-clap-sample`,
      `nix-flake-sample`, `devbox-citytime-sample` each built successfully
      under `strace -f -e trace=file` using their actual captured
      environment. See design.md Open Question 1 for the full findings,
      including the `$HOME/.cargo` gap found for nix/devbox (out of this
      change's scope).**
- [x] 0.2 Determine the `/nix` split: `/nix/store` read-only, `/nix/var`
      absent. Verify a real compile still works with `/nix/var` gone —
      this is the exact case the spike did not test. **Confirmed: zero
      `/nix/var` accesses in any of the three traced session builds**
      (design.md Open Question 1). The daemon-socket/profile accesses
      seen in an untrimmed trace belong to `flox activate`'s own
      provisioning-phase `nix build` calls, not the session.
- [x] 0.3 Establish the minimal `/dev` and whether `/proc` can be omitted
      entirely without `CLONE_NEWPID`. Read bubblewrap's own setup as a
      reference for what tooling breaks without (design.md M2).
      **Measured minimal sets recorded in design.md Open Question 1**:
      `/proc/self/{cgroup,exe,maps,statm}` +
      `/proc/sys/vm/overcommit_memory`; `/dev/{null,tty,urandom,fd/*}`.
- [x] 0.4 Decide open question 2: does this change take PID isolation too?
      Taking it closes the process-visibility gap and makes a private
      `/proc` meaningful; it also makes the keeper PID 1, which must reap.
      Decide before the mount plan is written, since `/proc` handling
      differs either way. **Decided: no** — the measured `/proc` need is
      five self-relative/global entries, nothing requiring process
      visibility isolation. Left to fleet's D2, consuming this change the
      way fleet already consumes `netns` (design.md Open Question 2).

## 1. The namespace primitive

- [x] 1.1 `enter_mount_namespace` alongside `fleet::netns`'s
      `enter_network_namespace`, adding `CLONE_NEWNS` to the `unshare`
      call already made in the keeper's `pre_exec`. No new privilege: the
      user namespace already created is what grants the `CAP_SYS_ADMIN`
      the mounts require. **Implemented as its own standalone `unshare()`
      call in `src/fleet/mount.rs`** (task group 1 is "the namespace
      primitive", independently testable, matching `netns`'s own
      precedent) — wiring it into `up.rs`'s actual `pre_exec` alongside
      `netns`'s call is task group 2's job, since mount isolation is
      unconditional while network isolation is conditional and the two
      cannot both call `unshare(CLONE_NEWUSER)` separately (a process may
      create a user namespace only once); see `mount.rs`'s own doc
      comment on `enter_mount_namespace`. **Found and fixed live, not
      anticipated**: a fresh user namespace has no uid/gid mapping, and a
      `tmpfs` write from an unmapped id fails with `EOVERFLOW`, not a
      permission error — `enter_mount_namespace` now also writes an
      identity `uid_map`/`gid_map` (mapping to `0`, the bubblewrap/
      `unshare --map-root-user` convention), with `setgroups` denied
      first as the kernel requires.
- [x] 1.2 Make propagation private (`MS_REC | MS_PRIVATE` on `/`) before
      any other mount. Without it the sandbox's mounts leak to the host,
      which is both a correctness bug and a mess to debug.
      **`make_propagation_private` in `src/fleet/mount.rs`.**
- [x] 1.3 A probe, matching `netns::probe`'s shape: attempt the real
      thing in a throwaway child rather than reading a sysctl, since
      seccomp, AppArmor and `max_user_namespaces` can each deny it
      independently. **`mount::probe` + hidden `__mount_probe`
      subcommand, and `tests/fleet_mount.rs` proves the deeper property —
      a mount made after entering does not leak to the host's own
      namespace — live, via `__mount_isolation_sim` (same "gate asks
      strictly less than the test asserts" discipline `fleet_netns.rs`
      documents). `doctor`'s existing namespace report now covers mount
      isolation too, from the same probe (design.md M4).**

## 2. The mount plan

- [x] 2.1 Construct the view from the compiled policy: project root
      read-write, provider-resolved paths read-only, the keeper's own
      system requirements, private `/tmp`. **`fleet::mount::construct_view`
      + `CapabilityPlan::resolved_grants` (a new shared resolver
      `policy/capability_set.rs` factors out of `to_capability_set`, so
      the view and Landlock's own grants are computed by the same code
      and cannot diverge). Live-verified via the hidden `__mount_view_probe`
      subcommand — a real `cargo build` succeeds inside the constructed,
      `pivot_root`ed view for all three closure-tier providers (flox,
      nix, devbox samples). Formalized as `tests/mount_view_e2e.rs`
      (devbox-citytime-sample: zero dependencies, no hook, so the test
      needs no network and no hook-splitting) so this stays a regression
      test rather than a one-off manual check.**

      **Three real bugs found only by running this, not by review:**
      an unprivileged `MS_REMOUNT|MS_RDONLY` on a bind-mounted *device
      node* (`/dev/urandom`) fails `EPERM` even though the identical call
      succeeds for directories and regular files moments earlier — fixed
      by skipping the read-only remount for char/block devices, since
      Landlock (applied after `pivot_root`) is the actual access-control
      layer regardless; a *fresh* `procfs` mount needs `CAP_SYS_ADMIN`
      in the user namespace that owns the **PID** namespace being shown,
      which this process's own namespace is not (task 0.4 deliberately
      did not take `CLONE_NEWPID`) — fixed by bind-mounting the host's
      existing `/proc` instead, which needs no such privilege and still
      resolves `/proc/self/*` correctly for every later session (procfs's
      self-symlink is superblock-relative, not tied to whichever process
      performed the bind; **this corrects design.md's original
      description of a narrow, fresh `/proc`** — without a PID namespace,
      `/proc` visibility is the full host list, the same as having no
      view at all for that one axis, not the 5-entry set open question 1
      measured as *needed*); and a dynamically-linked binary's own
      hard-coded ELF interpreter path (`/lib/ld-linux-aarch64.so.1` on
      this host) fails to exec with `ENOENT` because `resolved_grants`
      canonicalizes `/lib` to its real target `/usr/lib` before the view
      is built, so the `/lib` *symlink itself* — which the loader
      resolves inside the view, not through Landlock — never existed
      there. Fixed by `setup_merged_usr_compat`, recreating `/lib`,
      `/lib64`, `/bin`, `/sbin` as symlinks whenever the host has them
      (design.md M2 already named "merged-`/usr` symlinks" as bubblewrap
      know-how being forfeited; this is that cost, cashed out concretely
      rather than staying an abstract warning).
- [x] 2.2 **Include the sandbox's own proxy socket** (design.md M3). The
      state directory is baseline-denied for filesystem access, so masking
      it is the obvious move and would silently break egress for every
      isolated sandbox — the control and SSH sockets survive because they
      are inherited fds, the proxy socket does not because it is dialled
      by path. **Implemented as `construct_view`'s `proxy_socket`
      parameter, bind-mounted outside the generic `grants` loop. Verified
      live both ways: a real listening socket outside the project root is
      unreachable from inside the view without `--proxy-socket`, and
      reachable with it.**
- [x] 2.3 Fail closed if the view cannot be constructed; never fall back
      to the host namespace (design.md M4). **Structural at the primitive
      level (every step in `construct_view` is a plain `?`, so the first
      error stops the function and propagates — no branch could produce
      a working-but-weaker view) *and* now wired into `up` itself**:
      `up_process` probes `fleet::mount::probe` and fails with
      `UpError::Backend` before any listener is bound or state is
      written if this host cannot create unprivileged mount namespaces
      at all — the same "before anything is created" discipline the
      existing deny-overlap check follows — and `spawn_keeper`'s
      `pre_exec` now unconditionally calls
      `enter_mount_namespace_with_network` / `make_propagation_private` /
      `construct_view` for every sandbox (network isolation stays the
      conditional half of that same call, combined into one `unshare()`
      — a process may create a user namespace only once). A construction
      failure there propagates as an ordinary `Command::spawn()` error,
      `UpError::Keeper`; there is no fallback path. **Verified with the
      real `devcroft up` → `status` → `exec` → `down` CLI, end to end**
      (design.md, alongside this task) — not only the probe harness.
      **A behavioral consequence worth stating plainly**: every `up` now
      requires unprivileged mount namespaces, unconditionally — a host
      that previously ran devcroft under Landlock alone but cannot
      create one (where network isolation would have degraded
      gracefully) now has `up` refuse outright. This is exactly what
      design.md M4 calls for, not a bug, but it is a real, user-visible
      change from every devcroft release before this one.

## 3. Policy integration

- [ ] 3.1 Compile the view with origins, so a path present because the
      provider resolved it is distinguishable from one the manifest
      granted.
- [ ] 3.2 `policy --render` shows it — the "nothing reaches the backend
      that `--render` cannot show" invariant applies most strongly here,
      since a view decides what exists rather than what is permitted.
- [ ] 3.3 `doctor` reports availability alongside the existing
      network-namespace line, not as a second probe: both rest on the same
      unprivileged user namespace.

## 4. Tests

- [ ] 4.1 **Invert `tests/unix_socket_not_mediated.rs`.** That file
      currently asserts the *gap* — it passes because the hole is open,
      and its failure message names the three documents that must change
      when it closes. This change is what closes it; both tests should
      then assert refusal, and the docs they name must be corrected in the
      same commit.
- [ ] 4.2 A sandbox cannot reach the nix daemon socket, with a real
      daemon present on the host.
- [ ] 4.3 An isolated sandbox with `network.allow` still reaches its
      allowlisted hosts — the M3 regression, which would otherwise surface
      as a sandbox that starts healthy and has no network.
- [ ] 4.4 A real compile succeeds inside the view, per provider. This is
      what 0.1's measurement is for; without it the suite proves the gap
      closed and nothing about the sandbox still being usable.
- [ ] 4.5 **Verify the tests fail with the feature disabled**, the
      discipline this project applies to every namespace change since
      `fleet_netns.rs`'s skip guard was found to mask a broken feature.

## 5. Downstream

- [ ] 5.1 `add-linux-agent-fleet` task group 2 consumes this rather than
      implementing its own mount plan — the same relationship fleet
      already has to `fleet::netns`.
- [ ] 5.2 `sandbox-provisioning`'s P2a/P2b: the daemon-socket half of its
      claim becomes kernel-enforced rather than "devcroft declines to
      grant it". Correct that design.md note again when this lands.
- [ ] 5.3 `docs/known-gaps.md` and `docs/threat-model.md`: the AF_UNIX
      entry moves from open gap to closed, or to whatever remains true on
      macOS (open question 3).

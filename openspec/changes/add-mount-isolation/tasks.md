# Tasks — Mount Isolation

Ordered so the question that can invalidate the mount plan is answered
before the plan is written.

## 0. Measure the view before building it

> The spike that motivated this change masked *all* of `/nix`, which
> closed the daemon socket and would also have removed the toolchain. That
> is the whole difficulty of this change in one line: the correct view is
> narrow enough to close the gap and wide enough to compile. Guessing
> produces a sandbox that starts and fails at the first build.

- [ ] 0.1 For each closure-tier provider (flox, nix, devbox), record what
      a real build actually opens: run one under `strace -f -e trace=file`
      or equivalent and diff against the provider's resolved paths. The
      question is specifically whether anything outside the resolved
      closure is needed.
- [ ] 0.2 Determine the `/nix` split: `/nix/store` read-only, `/nix/var`
      absent. Verify a real compile still works with `/nix/var` gone —
      this is the exact case the spike did not test.
- [ ] 0.3 Establish the minimal `/dev` and whether `/proc` can be omitted
      entirely without `CLONE_NEWPID`. Read bubblewrap's own setup as a
      reference for what tooling breaks without (design.md M2).
- [ ] 0.4 Decide open question 2: does this change take PID isolation too?
      Taking it closes the process-visibility gap and makes a private
      `/proc` meaningful; it also makes the keeper PID 1, which must reap.
      Decide before the mount plan is written, since `/proc` handling
      differs either way.

## 1. The namespace primitive

- [ ] 1.1 `enter_mount_namespace` alongside `fleet::netns`'s
      `enter_network_namespace`, adding `CLONE_NEWNS` to the `unshare`
      call already made in the keeper's `pre_exec`. No new privilege: the
      user namespace already created is what grants the `CAP_SYS_ADMIN`
      the mounts require.
- [ ] 1.2 Make propagation private (`MS_REC | MS_PRIVATE` on `/`) before
      any other mount. Without it the sandbox's mounts leak to the host,
      which is both a correctness bug and a mess to debug.
- [ ] 1.3 A probe, matching `netns::probe`'s shape: attempt the real
      thing in a throwaway child rather than reading a sysctl, since
      seccomp, AppArmor and `max_user_namespaces` can each deny it
      independently.

## 2. The mount plan

- [ ] 2.1 Construct the view from the compiled policy: project root
      read-write, provider-resolved paths read-only, the keeper's own
      system requirements, private `/tmp`.
- [ ] 2.2 **Include the sandbox's own proxy socket** (design.md M3). The
      state directory is baseline-denied for filesystem access, so masking
      it is the obvious move and would silently break egress for every
      isolated sandbox — the control and SSH sockets survive because they
      are inherited fds, the proxy socket does not because it is dialled
      by path.
- [ ] 2.3 Fail closed if the view cannot be constructed; never fall back
      to the host namespace (design.md M4).

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

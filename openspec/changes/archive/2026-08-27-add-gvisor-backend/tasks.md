## 1. Backend module scaffold

- [x] 1.1 New top-level module `src/gvisor/` (`mod.rs`, `oci_spec.rs`,
      `runsc_command.rs`, `runner.rs` under `cfg(target_os = "linux")`,
      `session_backend.rs`), matching this crate's existing flat module
      style (`config`/`keeper`/`lifecycle`/`policy`/`provider`/`ssh`);
      wire it into `src/lib.rs`
- [x] 1.2 Platform selection: systrap by default; KVM only when
      `/dev/kvm` is present *and* accessible to the invoking user;
      ptrace is explicitly not supported

## 2. OCI bundle synthesis

- [x] 2.1 `oci_spec.rs`: build the OCI `config.json` from `CompiledPolicy`
      (not a parallel policy model) plus the resolved provider closure —
      minimal rootfs skeleton, store mount read-only, project root
      read-write, empty Linux capabilities. Pure JSON generation, no
      host dependency.
- [x] 2.2 Network mode selection per the corrected spec: `--network=none`
      when `network.default = "deny"` with no allowlist; `--network=host`
      when the manifest's `[network]` section grants egress
- [x] 2.3 Unit tests over the generated JSON for representative
      `CompiledPolicy` shapes (deny-all, egress-allowlist, filesystem
      grants, denied paths) — run on every host, no `runsc` required

## 3. runsc command assembly and runner

- [x] 3.1 `runsc_command.rs`: resolve the `runsc` binary (reuse
      `crate::paths::resolve_on_path`, the same helper `up.rs` already
      uses for `nono`); assemble args (`--platform`, `--rootless`,
      `--network`, `--root`); version probe
- [x] 3.2 `runner.rs` (`cfg(target_os = "linux")`): materialize the
      bundle under the existing `<state>/<name>/` dir — a persistent
      per-sandbox path, not a per-execution temp dir, since devcroft's
      sandboxes are long-lived; `runsc run` to start; `runsc exec` per
      session; `runsc kill` + `runsc delete` for `down`/`rm` teardown
- [x] 3.3 `up --recreate` rebuilds the bundle from the same manifest and
      lockfile

## 4. Session backend and host-side SSH placement

- [x] 4.1 `session_backend.rs`: `RunscExecBackend` implementing
      `add-hardened-tier`'s `SessionBackend` trait via
      `runsc exec <container> -- <argv>`, matching the existing
      `SpawnedSession` pty/stdio/signal/exit-code contract exactly
      (read `keeper/session.rs` and `keeper/pty.rs` in full before
      implementing — only signatures have been checked so far)
- [x] 4.2 Wire the `hardened` tier's `up` path (from
      `add-hardened-tier` task 3.3) to start the SSH/control listener
      host-side, reusing the existing keeper protocol/connection/pty
      code but backed by `RunscExecBackend` — no keeper process runs
      inside the sandbox at this tier
- [x] 4.3 Integration test, gated on `runsc` availability (self-skip
      otherwise, matching this repo's existing real-tooling test
      convention): an exec/shell session round-trip against a live
      gVisor sandbox, and an SSH round-trip through `devcroft proxy`
      (`tests/gvisor_hardened_e2e.rs`) — availability is a functional
      probe (`runsc --rootless --platform <p> do true`), matching
      `doctor`'s own check, not just binary presence; confirmed live to
      self-skip correctly in this devcontainer, where `runsc` is present
      but the platform doesn't actually work (see 10.3)

## 5. Landlock on Sentry (defense in depth) — REVERSED

**Live testing (add-flox-services task 6.5) found this whole layer makes
`runsc run` fail unconditionally under `--rootless`** — a `mount()` call
`runsc`'s own chroot setup issues returns `EPERM` under any active
Landlock ruleset, confirmed by elimination (even a ruleset granting `/`
full read-write fails identically), and Landlock cannot mediate
`mount()` in any ABI, so no grant could have fixed it. 5.1/5.2 were
implemented and shipped, but never exercised against a real unprivileged
user namespace until months later — the `landlock` crate dependency and
the ruleset application in `src/gvisor/runner.rs` have since been
**removed**; see design.md decision 4 for the full writeup. Left marked
`[x]` below rather than rewritten, since they describe what was actually
built and shipped at the time — the reversal is the record, not a retcon
of history.

- [x] 5.1 Add a `landlock` crate dependency — no existing devcroft code
      applies Landlock directly today; the process tier's enforcement
      lives entirely inside the external `nono` binary
- [x] 5.2 Apply a Landlock profile derived from `CompiledPolicy` to the
      Sentry process at sandbox start
- [x] 5.3 Test (gated on `runsc` availability): a compiled policy's
      filesystem grants bound what a compromised-Sentry scenario can
      reach; at minimum, a unit-level check that the profile compiled
      from a representative `CompiledPolicy` matches expectations

## 6. Provider-grant-to-mount verification

- [x] 6.1 At `up`, verify every provider read-only grant is
      representable as a bundle mount; fail at layer `provider` naming
      the path if one is not
- [x] 6.2 Unit test: a grant outside the mountable set fails loudly
      rather than silently widening the mount set or dropping the grant

## 7. doctor diagnostics

- [x] 7.1 `doctor_gvisor_backend()` in `src/bin/devcroft.rs`, mirroring
      `doctor_nix_provider()`'s shape: `runsc --version` presence and
      tested-range check
- [x] 7.2 Platform probe: `/dev/kvm` accessibility, a systrap smoke
      check (trivial `runsc do`-style probe or equivalent) rather than
      inferring usability from binary presence alone
- [x] 7.3 `[WARN]` when absent on Linux (noting it's only needed for
      `isolation = "hardened"` projects); `[FAIL]` with the fix named
      when present but unusable; permanent-platform-limitation message
      on macOS — per the `cli` delta spec's scenarios

## 8. Devcontainer

- [x] 8.1 `.devcontainer/Dockerfile`: pinned, checksum-verified `runsc`
      install, following the same convention as the existing
      `NONO_VERSION`/`FLOX_VERSION` pinned installs
- [x] 8.2 `.devcontainer/devcontainer.json`: explicitly revisit (not
      silently drop) the "No security-opt relaxations on purpose"
      comment — add exactly what rootless gVisor's unprivileged-userns
      creation needs, documented with the same specificity as the
      existing comments
- [x] 8.3 Note in the Dockerfile/devcontainer.json comments, matching
      the existing "UNVERIFIED beyond this build step" flox pattern:
      verifying this against a live `runsc` requires rebuilding the
      devcontainer in VS Code — it cannot be verified from inside the
      currently running container instance (no docker socket reachable,
      `unshare --user` currently fails `EPERM`)

## 9. Docs

- [x] 9.1 `docs/ssh-validation.md`: give its highest-priority
      listen-socket finding a tier-qualified answer — the hardened tier
      does *not* close it either, and why (design.md decision 1) —
      rather than leaving a reader to assume a new tier fixed it
- [x] 9.2 README: add the hardened tier to the isolation-tier story with
      its guarantee stated tier-qualified; correct the port-conflict
      story so it does not imply the hardened tier removes conflicts
- [x] 9.3 `docs/decisions.md`: record the rootless-vs-netstack tradeoff
      as a falsifiable rejection naming the property that fails, per
      that file's own convention, with the scoped-privilege alternative
      noted as revisitable; add the "revisit at hardened tier" note to
      the cgroup-limits rejection the proposal flags

## 10. Verification

- [x] 10.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
- [x] 10.2 `openspec validate --all` passes with this change's
      `tasks.md` added (currently 5/5 passing)
- [x] 10.3 Report e2e-against-live-runsc status honestly: now verified
      further than "compiles" (task 674d840 rebuilt the devcontainer with
      `runsc` installed), but still not fully live. `devcroft doctor`
      against the real, live-installed `runsc release-20260810.0`
      correctly reports `[FAIL] gvisor-backend`: the systrap platform's
      own re-exec into a fresh user namespace fails with
      `fork/exec /proc/self/exe: operation not permitted` — the exact
      `unshare --user` EPERM `.devcontainer/devcontainer.json`'s own
      comment already named as this container's platform boundary (no
      `security-opt` relaxation by default), not a new finding. `runner.rs`'s
      module doc's account stands: everything upstream of the userns wall
      (bundle synthesis, the Landlock ruleset, `runsc run` argument
      assembly) was exercised for real during development; the userns
      wall itself, `-detach` actually detaching, signal propagation into
      a sandboxed process, and the Landlock ruleset surviving into a
      started Sentry remain unconfirmed in *this* environment.
      `tests/gvisor_hardened_e2e.rs` (task 4.3) encodes this precisely:
      its availability probe matches `doctor`'s own functional check, and
      running it here self-skips with the same reason `doctor` reports —
      confirmed by actually running it, not just written to skip
      generically. Someone with a host that permits unprivileged userns
      creation (or this container's documented opt-in `security-opt`
      override) would exercise the remaining path today, no further code
      changes needed.

      **Update (add-flox-services task 6.5): that host arrived.** The
      `security-opt` override landed, `unshare --user` now succeeds
      here, and the userns wall this entry describes is gone — but two
      more real bugs sat behind it (a bundle whose mount destinations
      were never pre-created, and a relative `root.path` gVisor's
      symlink-escape guard can never match) and, past those, the
      Landlock ruleset this entry says "remains unconfirmed" turned out
      to be the thing actually blocking everything: it made `--rootless`
      fail with `EPERM` on its own `mount()` call, unconditionally, on
      any host. All four are fixed; the Landlock ruleset is removed
      rather than confirmed (see task group 5's note and design.md
      decision 4). A full `up` at `isolation = "hardened"`, `exec`, the
      SSH round trip, and a live `[services]` declaration all now work
      end to end — see `openspec/changes/add-flox-services/tasks.md`
      task 6.5 for the complete record.

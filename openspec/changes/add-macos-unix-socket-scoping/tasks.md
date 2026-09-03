# Tasks — macOS Unix Socket Scoping

## 0. The spike — before anything else, on real hardware

> Everything below this section assumes the spike confirms the mechanism. If it
> doesn't, stop and rewrite design.md's Decisions before writing any more code —
> do not adapt the implementation around a partial result and call the gap closed.
>
> **Outcome: the mechanism was confirmed, and one decision was overturned.** Run
> on macOS 15.7.4 (arm64) — the premise that this repository has no macOS host was
> itself wrong. S1 held exactly as read from the source; S2 did not, because macOS
> never dials the proxy over a unix socket at all. See design.md.

- [x] 0.1 On a real macOS host, with `nono` linked the same way devcroft links it
      (not `nono-cli`), build a `CapabilitySet` with `network.default = "deny"` and
      no unix-socket grants, apply it, and confirm `connect()` to a real, unrelated
      pathname unix socket on that host fails. Record the exact errno.
      → **Confirmed. `EPERM` (errno 1).** Measured against this host's real
      `/nix/var/nix/daemon-socket/socket` (`srw-rw-rw-`), with a control connect
      from outside the sandbox first to prove the daemon was live. Holds for both
      `block_network()` and `proxy_only(port)`. With the network left unrestricted
      the same connect **succeeds**, which is the gap itself, reproduced live.
- [x] 0.2 On the same host, add a `UnixSocketCapability`/`SocketScope` grant for one
      specific socket path, apply the same `CapabilitySet`, and confirm `connect()`
      to *that* socket now succeeds while an ungranted one still fails per 0.1.
      → **Confirmed, and confirmed to be scoped:** a socket sharing a directory with
      a granted one stays refused, so the grant matches the path it names rather
      than its parent.
- [x] 0.3 Inspect the actual Seatbelt profile text `nono` emits for both cases
      (`apply_auto` has a dry-run/render path on Linux already — confirm the macOS
      equivalent exists or capture it another way) and compare against design.md
      S1/S2's read of the source. Correct design.md if the emitted profile doesn't
      match what the source reading predicted.
      → **No render path exists on macOS** (`generate_profile` is private, `apply`
      is the only public entry point), so the profile was captured by interposing
      `sandbox_init` via `DYLD_INSERT_LIBRARIES`. The emitted text matches S1's
      reading exactly: `(deny network*)`, the two mDNSResponder carve-outs, and one
      added `(allow network-outbound (path "…"))` per grant. Recorded in design.md
      S1.
- [x] 0.4 Decide, from 0.1–0.3's actual results: does this change proceed as designed,
      need a different mechanism, or turn out to already be closed by something else
      entirely? Update design.md's Decisions section with the measured answer before
      task group 1 starts, the same way `add-mount-isolation`'s own task group 0
      corrected its design from a spike before the real implementation began.
      → **Decided: the capability was already enforced and simply never measured or
      claimed.** No policy-compilation change was needed for it. S1 stands as
      written; **S2 is superseded** (design.md carries the correction and keeps the
      original), which voids task group 1 below. What genuinely needed code was a
      prerequisite nobody had noticed: the crate did not compile on macOS at all.

## 0a. Prerequisite discovered by the spike — the crate did not build on macOS

> Not in the original plan, and load-bearing: `add-mount-isolation`'s graceful
> macOS degradation was written but had never been compiled, let alone run. Until
> this was fixed, no macOS behaviour of any kind could be observed.

- [x] 0a.1 Gate `src/fleet/mount.rs`'s Linux-only machinery (`construct_view` and
      its `mount(2)`/`pivot_root`/`umount2` helpers) behind `cfg(target_os =
      "linux")`, and add a non-Linux `construct_view` returning `Unsupported` — the
      shape `enter_mount_namespace`/`make_propagation_private` already had.
- [x] 0a.2 Gate the two Linux-only probe subcommands in `src/bin/devcroft.rs`
      (`__abstract_socket_probe`, which uses the Linux-only abstract namespace, and
      `__mount_isolation_sim`) with non-Linux counterparts that report unsupported
      rather than returning a result that would read as a measurement.
- [x] 0a.3 Gate `tests/abstract_socket_not_reachable.rs` at the crate level — the
      abstract namespace is a Linux concept with, per `nono`'s own doc, "no analog
      on macOS", so there is no equivalent property to port.
- [x] 0a.4 Verify: `cargo build`, `cargo clippy --all-targets` and `cargo test` all
      run clean on macOS. Done — clippy is warning-free and the suite runs.

## 1. Compile the proxy-socket grant — VOID for the proxy, REPLACED for the supervisor

> **Not implemented, deliberately.** The spike showed macOS reaches its egress
> proxy over TCP loopback, never over the proxy's unix socket: `relay` is `Some`
> only when `isolate_network` is, which needs a network namespace, which is
> Linux-only. `proxy_only(port)` already emits
> `(allow network-outbound (remote tcp "localhost:<port>"))`, captured live. A
> `UnixSocketCapability` here would be a security-shaped grant that can never
> fire. The spec requirement it was serving still holds and is still met — by the
> TCP grant, which is why that requirement is written as an outcome.

- [~] 1.1 Add the `UnixSocketCapability`/`SocketScope` grant for devcroft's own proxy
      socket to `policy/capability_set.rs`'s `to_capability_set`, macOS-only.
      → **Void.** No such grant is compiled; see above.
- [~] 1.2 Confirm the grant is *absent* when no proxy is running for that sandbox.
      → **Void by 1.1.** The scoping property the task was protecting is asserted
      directly instead, on the mechanism itself
      (`a_unix_socket_grant_admits_only_the_socket_it_names`).

> **But a grant of exactly this shape turned out to be needed for a different
> socket**, found by running the suite on macOS rather than by design: the same
> `(deny network*)` covers `bind(2)`, so the service supervisor could not create
> its own control socket even though it sits inside the granted project root, and
> every declared service died. That is the requirement's own stated failure mode
> ("starts, reports healthy, and does not work") arriving from the other side.

- [x] 1.3 Add `CompiledPolicy::unix_socket_bind` + `with_unix_socket_bind`, carried
      through `CapabilityPlan` (`#[serde(default)]`, so an older `profile.json`
      still reads back) and consumed in `to_capability_set` as a
      `UnixSocketCapability`/`ConnectBind` — macOS-only, since Landlock mediates
      no AF_UNIX operation and the mount view already covers Linux.
- [x] 1.4 Fold the service supervisor's socket in at `up`, under the same
      condition `prepare_services` uses, so a sandbox that will not start services
      carries no grant for a socket nothing creates.
- [x] 1.5 Render it. `policy --render` shows a `unix_socket.bind` section on both
      platforms, and `compile_with_provider_grants` reconstructs the entry from
      `Meta` for a running sandbox — the policy invariant is that nothing reaches
      the backend that `--render` cannot show, and the proxy port and provider
      grants are already folded in the same way.
- [x] 1.6 Verify: all 8 `tests/services_e2e.rs` tests pass on macOS, having failed
      with `bind: operation not permitted` before.

## 2. Tests

- [x] 2.1 Split `tests/unix_socket_not_mediated.rs`'s assertions by platform: Linux
      keeps asserting `ENOENT` (unchanged); add a macOS branch asserting `EPERM` from
      the network-deny rule specifically, matching design.md S3. Verify: the file
      compiles and the Linux branch still passes on this project's own devcontainer
      unchanged.
      → Done. Three macOS tests: the general property, the grant-scoping property,
      and the nix daemon socket. **The split needed more than a different errno**:
      the obvious arrangement was vacuous, passing with the deny rule removed,
      because `/tmp` is a symlink whose resolution the probe could not read. See
      design.md S3 for the two fixes and the teeth check that caught it.
- [x] 2.2 A macOS-only integration test exercising 1.1/1.2 through the real `up` path
      (a sandbox with `network.allow`, confirming egress still works) — the macOS
      analogue of `tests/mount_view_e2e.rs`'s proxy-socket test.
      → **Reduced in scope, because 1.1/1.2 are void.** There is no macOS-only
      compiled grant to exercise, so the egress-through-proxy path this would have
      asserted is the ordinary `proxy_only` TCP path already covered by
      `tests/proxy_up.rs` and `tests/egress_proxy_e2e.rs`, which now actually run on
      macOS for the first time (0a). What was specific to this change — that a
      grant admits one socket and not its neighbour — is asserted directly in 2.1
      rather than through `up`.

## 3. Make the claim, only now

- [x] 3.1 `docs/known-gaps.md`: correct the macOS residual note in the AF_UNIX entry
      from "no measured equivalent" to closed — only after task group 0's spike
      confirms it, citing the spike's own result as evidence.
      → Done, with all three residual limits named: the guarantee is scoped to
      deny-default sandboxes, it is reachability and not visibility, and a
      `filesystem.allow` grant opens a socket on Linux but not on macOS.
- [x] 3.2 `docs/threat-model.md`: same correction, same precondition.
      → Done, stated as the part the capability matrix cannot carry: on macOS this
      boundary is conditional on the manifest's own network mode, where on Linux it
      is not, so the same manifest yields a weaker boundary on macOS.
- [x] 3.3 `src/backend_capabilities.rs`'s `pathname-unix-sockets` entry: macOS status
      moves from `Unsupported` to `Enforced` (or `EnforcedWithNamedDegradation`, if
      2.1/2.2 surface a partial case), evidence citing the new test(s) — only after
      task group 0.
      → **`EnforcedWithNamedDegradation`**, since 2.1 did surface partial cases. Both
      degradations are named in the entry itself, per that status's own contract.

## 4. Surfaces that claimed the Linux mechanism on macOS

> Found while verifying 3.x. The spec's third requirement covers `doctor` "or any
> other user-facing surface", and three of them described a filesystem view and a
> network namespace that do not exist on this platform.

- [x] 4.1 `devcroft doctor`'s `namespaces` line said `up` "fails outright for mount
      isolation" — untrue on macOS, where it warns and proceeds. Now platform-split,
      and points at the `pathname-unix-sockets` entry for what still holds.
- [x] 4.2 `up`'s mount-isolation warning said a world-accessible unix socket
      "remains reachable" unconditionally. Now depends on the sandbox's own network
      mode, which is the thing that actually determines it.
- [x] 4.3 `policy --render` described a mount view every sandbox gets, and a
      network namespace it does not get, on a platform with neither. Both now
      platform-accurate; `render_explains_the_filesystem_view` asserts per platform
      rather than sharing one weak assertion.

## 5. Pre-existing macOS breakage surfaced by 0a, and what was done with it

> None of this is caused by this change; all of it became *visible* for the first
> time when the crate started compiling on macOS. Recorded here rather than
> folded silently into the diff, because two of these are real defects and four
> are published gaps.

**Fixed, because they were real defects:**

- [x] 5.1 `lifecycle/hooks.rs` spawned hooks with a bare `"sh"`, resolved by the
      *sandbox's* `PATH` — CLAUDE.md names this as load-bearing for three call
      sites, and hooks were an uncaught fourth. Now uses the closure-resolved
      absolute shell from `Meta`. It failed loudly on macOS and silently worked on
      Linux, which is exactly why the invariant exists.
- [x] 5.2 The services supervisor socket grant (task group 1 above).

**Published as gaps, with the tests gated to point at them rather than weakened:**

- [x] 5.3 `docs/known-gaps.md`: host binaries execute on macOS even at ungranted
      paths (`(allow process-exec*)` is unconditional), so
      `own-policy-baseline`'s "host toolchain is denied" is Linux-only.
- [x] 5.4 `docs/known-gaps.md`: a C toolchain from a closure cannot link on macOS
      — the nix linker wrapper reads `/dev/fd/63` and the baseline grants no
      `/dev/fd`. Widening the baseline belongs to `own-policy-baseline`.
- [x] 5.5 `docs/known-gaps.md`: interactive pty sessions are refused on macOS —
      `openpty()` must open the pty *slave* and only the master is granted.
      Measured directly, not inferred. Needs an upstream rule; `devcroft shell`
      and SSH pty sessions do not work on macOS today.
- [x] 5.6 `docs/known-gaps.md`: a grant does not cover the symlinked spelling of
      its own path (`/tmp`, `/var`), so a project under `$TMPDIR` is refused
      through the path it was granted by.
- [x] 5.7 `docs/known-gaps.md` + capability matrix: `network.ports` is
      all-or-nothing on macOS (Seatbelt cannot filter bind by port). The matrix
      claimed `enforced` while carrying its own note that nobody had measured it;
      now `enforced (degraded)`, and `policy::degraded` warns at `up`.

**Test-fixture bugs, fixed in place:**

- [x] 5.8 Project roots built from `std::env::temp_dir()` now canonicalize (the
      5.6 gap made them unusable as granted paths); the services tests use a
      short `/tmp` root, because macOS's `$TMPDIR` overflows `sun_path` and
      devcroft's own guard — correctly — refused to start.
- [x] 5.9 `tests/host_port_reachability.rs` ran `devcroft exec` from the *crate*
      root, so the session's cwd was a directory the sandbox does not grant and
      the server never started. Its isolated twin asserts *un*reachability and was
      therefore passing for the wrong reason; both now run from the project.
- [x] 5.10 The nix flake fixture hardcoded `-linux` in its system double;
      `tests/ssh_channels.rs` assumed the host's `rsync` would be reachable inside
      a sandbox that installs only `bash`/`coreutils`.
- [x] 5.11 `tests/egress_proxy_e2e.rs` binds `127.0.0.3`/`127.0.0.4`, which macOS
      does not assign by default; now skips naming the `ifconfig lo0 alias`
      remedy instead of failing.
- [x] 5.12 rsync over SSH: skipped on macOS and **not attributed** — reduced to a
      minimal case where `rsync` copying one local file to another inside a
      sandbox fails `change_dir ... Operation not permitted`, while `cd` and
      `touch` in that same directory succeed and scp/sftp over the same channel
      pass.

# Tasks — macOS Unix Socket Scoping

## 0. The spike — before anything else, on real hardware

> Everything below this section assumes the spike confirms the mechanism. If it
> doesn't, stop and rewrite design.md's Decisions before writing any more code —
> do not adapt the implementation around a partial result and call the gap closed.

- [ ] 0.1 On a real macOS host, with `nono` linked the same way devcroft links it
      (not `nono-cli`), build a `CapabilitySet` with `network.default = "deny"` and
      no unix-socket grants, apply it, and confirm `connect()` to a real, unrelated
      pathname unix socket on that host fails. Record the exact errno.
- [ ] 0.2 On the same host, add a `UnixSocketCapability`/`SocketScope` grant for one
      specific socket path, apply the same `CapabilitySet`, and confirm `connect()`
      to *that* socket now succeeds while an ungranted one still fails per 0.1.
- [ ] 0.3 Inspect the actual Seatbelt profile text `nono` emits for both cases
      (`apply_auto` has a dry-run/render path on Linux already — confirm the macOS
      equivalent exists or capture it another way) and compare against design.md
      S1/S2's read of the source. Correct design.md if the emitted profile doesn't
      match what the source reading predicted.
- [ ] 0.4 Decide, from 0.1–0.3's actual results: does this change proceed as designed,
      need a different mechanism, or turn out to already be closed by something else
      entirely? Update design.md's Decisions section with the measured answer before
      task group 1 starts, the same way `add-mount-isolation`'s own task group 0
      corrected its design from a spike before the real implementation began.

## 1. Compile the proxy-socket grant

- [ ] 1.1 Add the `UnixSocketCapability`/`SocketScope` grant for devcroft's own proxy
      socket to `policy/capability_set.rs`'s `to_capability_set`, macOS-only
      (`#[cfg(target_os = "macos")]`) — the sibling of what `fleet::mount::
      construct_view`'s `proxy_socket` parameter does on Linux. Verify by rendering
      the compiled `CapabilitySet` (or the emitted profile text, once 0.3 establishes
      how to inspect it) and confirming the grant is present exactly when a proxy is
      running for that sandbox.
- [ ] 1.2 Confirm the grant is *absent* when no proxy is running for that sandbox
      (`network.default = "allow"`, or `deny` with no `network.allow`) — mirroring
      `filesystem-view`'s own "another sandbox's proxy socket" scenario. Verify with
      a unit test on the compiled `CapabilitySet`, not requiring a macOS host for
      this half (the grant's presence/absence is host-independent to check).

## 2. Tests

- [ ] 2.1 Split `tests/unix_socket_not_mediated.rs`'s assertions by platform: Linux
      keeps asserting `ENOENT` (unchanged); add a macOS branch asserting `EPERM` from
      the network-deny rule specifically, matching design.md S3. Verify: the file
      compiles and the Linux branch still passes on this project's own devcontainer
      unchanged.
- [ ] 2.2 A macOS-only integration test exercising 1.1/1.2 through the real `up` path
      (a sandbox with `network.allow`, confirming egress still works) — the macOS
      analogue of `tests/mount_view_e2e.rs`'s proxy-socket test. Cannot be verified on
      this project's Linux-only CI/devcontainer; gated so it skips cleanly rather than
      failing where it can't run, matching every other platform-gated test in this
      project (`#[cfg(target_os = "macos")]` or a runtime host check, whichever this
      project's existing macOS-gated code — if any — already establishes as the
      convention).

## 3. Make the claim, only now

- [ ] 3.1 `docs/known-gaps.md`: correct the macOS residual note in the AF_UNIX entry
      from "no measured equivalent" to closed — only after task group 0's spike
      confirms it, citing the spike's own result as evidence, the same discipline
      `add-mount-isolation` and `add-backend-capabilities` both applied to their own
      corrections.
- [ ] 3.2 `docs/threat-model.md`: same correction, same precondition.
- [ ] 3.3 `src/backend_capabilities.rs`'s `pathname-unix-sockets` entry: macOS status
      moves from `Unsupported` to `Enforced` (or `EnforcedWithNamedDegradation`, if
      2.1/2.2 surface a partial case), evidence citing the new test(s) — only after
      task group 0.

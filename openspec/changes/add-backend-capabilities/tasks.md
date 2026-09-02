# Tasks — Backend Capabilities

## 0. Decide the shape

- [ ] 0.1 Choose format and location (design.md open question 1): compiled-in
      Rust versus data file. Decide before writing entries — retrofitting the
      other way means rewriting all of them.
- [ ] 0.2 Choose granularity (open question 2): user-visible capability,
      enforcement mechanism, or nested.
- [ ] 0.3 Fix the vocabulary in one place, so `unverified` and `not-adopted`
      cannot be spelled two ways.

## 1. Populate from what is actually known

Each entry needs its evidence, and writing them is expected to surface claims
nobody can support — that is the point, not a setback.

- [ ] 1.1 Filesystem policy (Landlock/Seatbelt path grants).
- [ ] 1.2 Network block, and loopback port grants (`network.ports`).
- [ ] 1.3 Domain filtering. **Linux `enforced`** — evidence:
      `tests/egress_proxy_e2e.rs`. **macOS `unverified`** — the honest status;
      `policy::degraded` currently asserts cooperative, the library's doc
      comment suggests enforced, and nobody has run it.
- [ ] 1.4 Signal isolation — the one library knob devcroft actually sets.
- [ ] 1.5 Process-info isolation: `not-adopted`. devcroft never configures
      `ProcessInfoMode` and silently inherits the library's default.
      **IPC mode is a separate entry, and the premise this task started
      from was wrong — corrected here, not left standing.** This task
      originally read "IpcMode: not-adopted... a sandbox reaches every
      abstract unix socket... one method call devcroft already has access
      to", reasoning from "we never call `set_ipc_mode`" without checking
      what the *unset* default resolves to. It does not need calling:
      `IpcMode::SharedMemoryOnly` is `nono`'s own `#[default]`
      (`capability.rs`), and `requested_scopes()`
      (`sandbox/linux.rs`) requests Landlock's `Scope::AbstractUnixSocket`
      whenever `ipc_mode() == SharedMemoryOnly` — true on every devcroft
      sandbox, today, with zero code change. **Verified live, not just
      traced**: `__abstract_socket_probe` applies devcroft's real,
      unmodified `CapabilitySet` against a real abstract socket and gets
      `EPERM`. Record IPC mode's abstract-socket scoping as **`enforced`
      on Linux with Landlock ABI V6+** (evidence:
      `__abstract_socket_probe`, exercised by
      `tests/abstract_socket_not_reachable.rs`), **`unsupported` on
      older Linux kernels** (no scoping ABI to request), and
      **`unverified` on macOS** (Seatbelt has no equivalent examined).
      This is the second half of the gap `docs/known-gaps.md` records,
      now corrected there to say *closed* rather than open — the pathname
      half needed `add-mount-isolation`'s mount namespace; the abstract
      half needed nothing, and `add-backend-capabilities` is what noticed
      that rather than what fixed it. Originally found by reading
      `sandlock` (see `docs/prior-art.md`), which uses the same scoping
      deliberately; the matrix work here is what caught that the
      "unadopted" framing was itself wrong.
- [ ] 1.6 Resource limits: `not-adopted`. The library's `ResourceLimits` is a
      declaration only — rendering it to cgroups lived in the CLI devcroft
      stopped depending on (confirmed in `add-linux-agent-fleet` task 0).
- [ ] 1.7 Snapshot/`undo`, keystore: `not-adopted`. Audit: `not-adopted`
      today, **with a named consumer** — `add-agent-interaction`'s durable
      record should be `nono`'s append-only NDJSON with a rolling chain hash
      and Merkle commit, rather than a second log format.
- [ ] 1.7b Credential brokering (`nono-proxy` reverse mode, `jwt_phantom`):
      `not-adopted`; `add-egress-proxy` E6 proposes adopting the crate for
      this and `add-agent-workload` specifies the capability, but the crate
      itself is not yet taken — the auth gap that first motivated looking at
      it was closed directly in devcroft's own proxy instead (task group 4a),
      so this capability alone is what remains to justify adoption, not the
      thing that made it urgent. Record the `jwt_phantom`
      detail as evidence of why "we could write this ourselves" is a weaker
      argument than it sounds: a consumer that validates token *structure*
      rejects an opaque placeholder before any request is made, and that is
      not a thing anyone designs for in advance.
- [ ] 1.7c L7 endpoint policy (`SERVICE:METHOD:PATH`): `not-adopted`, **no
      consumer yet**. Recorded because the matrix's job is showing the gap
      between offered and used, and "allow github.com, GET only" is a real
      want no devcroft change currently expresses.
- [ ] 1.7d TLS interception, SPIFFE, AWS routing: `not-adopted` **by
      decision, not by omission** — `add-egress-proxy` refuses the first
      explicitly and the others are out of scope. The distinction matters if
      `nono-proxy` is ever adopted: these would arrive as capabilities of
      that dependency, so a future reader must be able to tell "we chose not
      to" from "nobody got to it".
- [ ] 1.8 `supervisor` (runtime capability approval): `not-adopted` **today,
      with a named consumer** — `add-agent-interaction` adopts it to turn a
      policy denial into a request an operator can answer. Worth recording as
      an example of what this matrix is for: the mechanism has been shipped in
      the library the whole time, devcroft solved nothing with it, and nothing
      anywhere said so.
- [ ] 1.9 Per-agent network namespaces: Linux `enforced`, evidence
      `tests/fleet_netns.rs`; macOS `unsupported`.
- [ ] 1.10 Inter-sandbox process visibility: `not-adopted` outside fleet — the
      known gap the README already publishes.

## 2. Surface it

- [ ] 2.1 `doctor` reports declared capabilities against this host, with the
      declared-versus-available distinction explicit.
- [ ] 2.2 Do not probe the host for `not-adopted` capabilities; their absence
      is not a host deficiency.

## 3. Make the prose defer

- [ ] 3.1 README: replace capability claims with a pointer. The "Known gaps"
      list is the main duplication.
- [ ] 3.2 `docs/threat-model.md`: its pointer stops dangling; check the
      surrounding prose does not restate what the matrix now carries.
- [ ] 3.3 `docs/decisions.md`: same, for the network-filtering entries in
      particular — that is where a corrected-in-place claim already lives.
- [ ] 3.4 `policy::degraded`'s module doc: point at the matrix for macOS
      domain filtering rather than asserting a status nobody measured.

## 4. Close the dependencies this was blocking

- [ ] 4.1 `remove-gvisor-backend` task 4.1: mark resolved, since the rewrite it
      was waiting for is this change.
- [ ] 4.2 `add-linux-agent-fleet`: express its required capabilities (`fleet`,
      `service_ports`, `resource_limits`, `process_isolation`) as declarations
      against this matrix rather than as assumptions.

## 5. Keep it honest

- [ ] 5.1 A test asserting every entry has evidence, so an entry cannot be
      added as `enforced` with nothing behind it.
- [ ] 5.2 Decide whether `unverified` warns in CI (open question 3). Default to
      visible-not-failing, to avoid pressure toward thin `enforced` claims.

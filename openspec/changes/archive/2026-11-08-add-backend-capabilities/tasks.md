# Tasks — Backend Capabilities

## 0. Decide the shape

- [x] 0.1 Choose format and location (design.md open question 1): compiled-in
      Rust versus data file. Decide before writing entries — retrofitting the
      other way means rewriting all of them. **Decided: compiled-in
      (`src/backend_capabilities.rs`)**, matching design.md's own lean —
      the requirement that a change updating a capability update the
      declaration in the same change argues for it living where the code
      does, where `cargo build` and review both see it.
- [x] 0.2 Choose granularity (open question 2): user-visible capability,
      enforcement mechanism, or nested. **Decided: per user-visible
      capability** — one entry per thing a `doctor`/README reader cares
      about; the mechanism is what a given entry's `evidence` cites.
- [x] 0.3 Fix the vocabulary in one place, so `unverified` and `not-adopted`
      cannot be spelled two ways. **`backend_capabilities::Status`** — a
      closed `enum`, not `String`; `Display` renders the five spec-named
      words exactly.

## 1. Populate from what is actually known

Each entry needs its evidence, and writing them is expected to surface claims
nobody can support — that is the point, not a setback.

- [x] 1.1 Filesystem policy (Landlock/Seatbelt path grants). **`filesystem-policy`
      entry**: `enforced` both platforms.
- [x] 1.2 Network block, and loopback port grants (`network.ports`).
      **`network-block-and-ports` entry**: `enforced` both platforms.
- [x] 1.3 Domain filtering. **Linux `enforced`** — evidence:
      `tests/egress_proxy_e2e.rs`. **macOS `unverified`** — the honest status;
      `policy::degraded` currently asserts cooperative, the library's doc
      comment suggests enforced, and nobody has run it. **`domain-filtering`
      entry, exactly as specified.**
- [x] 1.4 Signal isolation — the one library knob devcroft actually sets.
      **`signal-isolation` entry**: `enforced-with-named-degradation` on
      Linux (ABI V6+ dependency named in the evidence), `unverified` on
      macOS.
- [x] 1.5 Process-info isolation: `not-adopted`. devcroft never configures
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
      `EPERM`. **`process-info-isolation` entry**: `not-adopted` both
      platforms, as originally specified — its default already denies
      rather than grants, so nothing was silently widened.
      **`abstract-unix-sockets` entry** (split out, corrected): `enforced-
      with-named-degradation` on Linux (evidence:
      `tests/abstract_socket_not_reachable.rs`, degrades on pre-V6
      kernels), `unverified` on macOS. This is the second half of the gap
      `docs/known-gaps.md` records, now corrected there to say *closed*
      rather than open — the pathname half needed `add-mount-isolation`'s
      mount namespace; the abstract half needed nothing, and
      `add-backend-capabilities` is what noticed that rather than what
      fixed it. Originally found by reading `sandlock` (see
      `docs/prior-art.md`), which uses the same scoping deliberately; the
      matrix work here is what caught that the "unadopted" framing was
      itself wrong. **Also added, not originally listed**:
      `pathname-unix-sockets` (`enforced` Linux / `unsupported` macOS,
      evidence `tests/unix_socket_not_mediated.rs`) — the AF_UNIX gap's
      other half belongs in this matrix too, and leaving it out would
      have made the abstract-socket entry read as the whole story.
- [x] 1.6 Resource limits: `not-adopted`. The library's `ResourceLimits` is a
      declaration only — rendering it to cgroups lived in the CLI devcroft
      stopped depending on (confirmed in `add-linux-agent-fleet` task 0).
      **`resource-limits` entry, as specified.**
- [x] 1.7 Snapshot/`undo`, keystore: `not-adopted`. Audit: `not-adopted`
      today, **with a named consumer** — `add-agent-interaction`'s durable
      record should be `nono`'s append-only NDJSON with a rolling chain hash
      and Merkle commit, rather than a second log format. **Three separate
      entries: `snapshot-and-undo`, `keystore`, `audit-log`**, all
      `not-adopted`, matching the granularity decision (0.2) rather than
      collapsing them into one.
- [x] 1.7b Credential brokering (`nono-proxy` reverse mode, `jwt_phantom`):
      `not-adopted`; `add-egress-proxy` E6 proposes adopting the crate for
      this and `add-agent-workload` specifies the capability, but the crate
      itself is not yet taken — the auth gap that first motivated looking at
      it was closed directly in devcroft's own proxy instead (task group 4a),
      so this capability alone is what remains to justify adoption, not the
      thing that made it urgent. **`credential-brokering` entry, as
      specified**, `jwt_phantom` detail included in the evidence.
- [x] 1.7c L7 endpoint policy (`SERVICE:METHOD:PATH`): `not-adopted`, **no
      consumer yet**. Recorded because the matrix's job is showing the gap
      between offered and used, and "allow github.com, GET only" is a real
      want no devcroft change currently expresses. **`l7-endpoint-policy`
      entry, as specified.**
- [x] 1.7d TLS interception, SPIFFE, AWS routing: `not-adopted` **by
      decision, not by omission** — `add-egress-proxy` refuses the first
      explicitly and the others are out of scope. The distinction matters if
      `nono-proxy` is ever adopted: these would arrive as capabilities of
      that dependency, so a future reader must be able to tell "we chose not
      to" from "nobody got to it". **Three entries** (`tls-interception`,
      `spiffe-identity`, `aws-request-routing`), matching 0.2's granularity
      rather than one combined "nono-proxy extras" entry.
- [x] 1.8 `supervisor` (runtime capability approval): `not-adopted` **today,
      with a named consumer** — `add-agent-interaction` adopts it to turn a
      policy denial into a request an operator can answer. Worth recording as
      an example of what this matrix is for: the mechanism has been shipped in
      the library the whole time, devcroft solved nothing with it, and nothing
      anywhere said so. **`runtime-capability-approval` entry, as specified.**
- [x] 1.9 Per-agent network namespaces: Linux `enforced`, evidence
      `tests/fleet_netns.rs`; macOS `unsupported`. **`per-agent-network-
      namespace` entry, as specified.**
- [x] 1.10 Inter-sandbox process visibility: `not-adopted` outside fleet — the
      known gap the README already publishes. **`inter-sandbox-process-
      visibility` entry, as specified** — unchanged by `add-mount-
      isolation`'s deliberate choice not to take a PID namespace (that
      change's own design.md Open Question 2).

## 2. Surface it

- [x] 2.1 `doctor` reports declared capabilities against this host, with the
      declared-versus-available distinction explicit. **`doctor_backend_
      capabilities` in `src/bin/devcroft.rs`**, wired into `cli_doctor`.
      Live-verified: `pathname-unix-sockets` and `abstract-unix-sockets`
      both report "enforced..., and available on this host" on this
      devcontainer.
- [x] 2.2 Do not probe the host for `not-adopted` capabilities; their absence
      is not a host deficiency. **Structural, not a per-entry check**:
      `doctor_backend_capabilities` only calls `probe_here()` inside the
      `Enforced`/`EnforcedWithNamedDegradation` match arm; `NotAdopted`
      entries never reach it. Backed by
      `not_adopted_entries_carry_no_host_probe`, which asserts no
      `NotAdopted` entry even *carries* a probe function to call.

## 3. Make the prose defer

- [x] 3.1 README: replace capability claims with a pointer. The "Known gaps"
      list is the main duplication. **Done** — the Status section's stale
      "unix sockets bypass the policy entirely" claim (both halves are now
      closed) is removed; the section now points at `devcroft doctor` and
      `docs/known-gaps.md` and states the defer-to-the-matrix rule
      explicitly.
- [x] 3.2 `docs/threat-model.md`: its pointer stops dangling; check the
      surrounding prose does not restate what the matrix now carries.
      **Done** — the "declared data, not prose" pointer now names the real
      module and `doctor`, not a change that didn't exist yet; the AF_UNIX
      bullet is trimmed to threat-model reasoning (is this in scope) with
      status deferred to the matrix.
- [x] 3.3 `docs/decisions.md`: same, for the network-filtering entries in
      particular — that is where a corrected-in-place claim already lives.
      **Done** — "Cooperative network filtering" and "No inter-sandbox
      process isolation (MVP)" both keep their history (design.md C4: a
      matrix cannot carry *why*) and now point at the matrix for current
      status rather than asserting it inline.
- [x] 3.4 `policy::degraded`'s module doc: point at the matrix for macOS
      domain filtering rather than asserting a status nobody measured.
      **Done** — added a closing pointer to `backend_capabilities`'s
      `domain-filtering` entry; the existing `Unverified`-on-macOS
      reasoning stays as the history.

## 4. Close the dependencies this was blocking

- [x] 4.1 `remove-gvisor-backend` task 4.1: mark resolved, since the rewrite it
      was waiting for is this change. **Already marked done in that
      change's own tasks.md** ("Unblocked and done — by writing the
      change rather than rewriting it") — nothing further needed; this
      task confirms rather than performs an action.
- [x] 4.2 `add-linux-agent-fleet`: express its required capabilities (`fleet`,
      `service_ports`, `resource_limits`, `process_isolation`) as declarations
      against this matrix rather than as assumptions. **Done** — that
      change's own proposal.md now names the real entries:
      `service_ports` → `per-agent-network-namespace` (already `enforced`
      on Linux, fleet's own `fleet::netns` slice is the evidence);
      `resource_limits` → `resource-limits` (`not-adopted`, fleet's own
      remaining work); `process_isolation` → `inter-sandbox-process-
      visibility` (`not-adopted`, fleet's D2). The placeholder `fleet`
      name is dropped — it never mapped to one capability.

## 5. Keep it honest

- [x] 5.1 A test asserting every entry has evidence, so an entry cannot be
      added as `enforced` with nothing behind it. **Four tests in
      `backend_capabilities`'s own `#[cfg(test)]` module**:
      `every_entry_has_nonempty_evidence_on_both_platforms`,
      `degraded_entries_explain_the_degradation_in_their_own_evidence`,
      `not_adopted_entries_carry_no_host_probe` (2.2's own structural
      guarantee, tested directly), `names_are_unique`.
- [x] 5.2 Decide whether `unverified` warns in CI (open question 3). Default to
      visible-not-failing, to avoid pressure toward thin `enforced` claims.
      **Decided: visible-not-failing**, matching design.md's own lean —
      `doctor_backend_capabilities` reports every status, `Unverified`
      included, as `[INFO]`, never `[FAIL]`; `cli_doctor`'s pass/fail
      verdict is unaffected by anything in this matrix.

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
- [ ] 1.5 Process-info isolation and IPC mode: `not-adopted`. devcroft never
      configures either and silently inherits the library's defaults.
- [ ] 1.6 Resource limits: `not-adopted`. The library's `ResourceLimits` is a
      declaration only — rendering it to cgroups lived in the CLI devcroft
      stopped depending on (confirmed in `add-linux-agent-fleet` task 0).
- [ ] 1.7 Snapshot/`undo`, keystore, audit: `not-adopted`.
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

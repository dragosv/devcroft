# Tasks — Manifest Key Liveness

## 1. The check

- [ ] 1.1 A test that walks `config::validate::SECTIONS`, derives each key's
      Rust field path, and requires a reference outside `src/config/`.
- [ ] 1.2 `DELIBERATELY_INERT` as one list of (key, reason) in the test file
      (D2). Seed it with `sandbox.isolation` and the reason
      `remove-gvisor-backend` gives.
- [ ] 1.3 Strip string literals before matching, reusing the approach the
      existing name-branching lint already needed — that lint flagged *itself*
      before it did so.

## 2. Prove it can fail

- [ ] 2.1 Remove a real consumer — `manifest.network.allow`'s, say — and
      confirm the check fails naming that key. A liveness check that cannot
      fail is the thing it exists to replace.
- [ ] 2.2 Confirm a key in `DELIBERATELY_INERT` passes, and that removing it
      from that list makes the same key fail. The escape hatch has to be load-
      bearing, not decorative.

## 3. Say what it does not check

- [ ] 3.1 Record in the test's own doc comment that this proves *a consumer
      exists*, not that the key works. A reader who trusts it further than that
      will be wrong in exactly the way the three found keys were wrong.
- [ ] 3.2 `docs/implementation-log.md`: the three keys, how each was found, and
      why every automated signal reported health while they were dead.

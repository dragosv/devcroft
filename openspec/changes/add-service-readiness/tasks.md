# Tasks: add-service-readiness

## 0. Measurement gate

- [ ] 0.1 Confirm devenv's schema against a project that sets every
      field, reading back `devenv eval processes` — the shape in
      design.md comes from devenv's source, and source and behaviour are
      not the same evidence.
- [ ] 0.2 Confirm process-compose accepts the `readiness_probe` shape
      devcroft will emit, against the real binary rather than its docs.
- [ ] 0.3 **Does `http.get` work from inside a sandbox?** A loopback
      probe from the supervisor to a port the service binds. If it needs
      a grant the manifest does not already have, that changes what this
      change must document.
- [ ] 0.4 Measure what process-compose reports for a probe that never
      passes, since decision 4 rests on it being visible rather than
      fatal.

## 1. `ServiceDecl` gains readiness

- [ ] 1.1 `Readiness` with a `Probe` enum (design.md decision 1),
      supervisor-neutral.
- [ ] 1.2 `render_config` emits `readiness_probe`.
- [ ] 1.3 **The dependency condition becomes conditional** (decision 3):
      `process_healthy` where the target declares readiness,
      `process_started` where it does not. Remove the comment that says
      readiness is refused — it stops being true here.
- [ ] 1.4 A service declaring no readiness renders byte-identically:
      extend the existing flox golden rather than adding a parallel one.

## 2. devenv maps instead of refusing

- [ ] 2.1 Map `exec` and `http.get`, with `initial_delay`, `period`,
      `probe_timeout`, `success_threshold`, `failure_threshold`.
- [ ] 2.2 Refuse `notify` and the overall `timeout` by name, saying what
      each would have meant (decision 2).
- [ ] 2.3 Remove `ready` from the refused-fields list, leaving the other
      seven intact — a test asserts the rest still refuse, so this does
      not quietly widen what is accepted.

## 3. Tests

- [ ] 3.1 Unit: both probe forms map; `notify` and `timeout` refuse by
      name.
- [ ] 3.2 Unit: the dependency condition is `process_healthy` only when
      the target declares readiness.
- [ ] 3.3 E2E: a service with a slow probe is reported started-then-ready,
      and a dependent starts **after** ready — observed by order, not by
      reading the config.
- [ ] 3.4 E2E: a probe that never passes leaves `up` successful and the
      service visibly not ready (decision 4).
- [ ] 3.5 Regression: flox services unchanged.

## 4. Sample and documentation

- [ ] 4.1 `samples/devenv-services-sample`: the api process gets a real
      probe, and the README explains why its dependent now waits.
- [ ] 4.2 `docs/decisions.md`: devbox's entry loses readiness as a listed
      cost, since it now exists. The contract question is untouched and
      stays the deciding one.
- [ ] 4.3 `docs/implementation-log.md`: what group 0 measured.

## 5. Verification

- [ ] 5.1 build, clippy, fmt, doc clean.
- [ ] 5.2 Full suite, skips reviewed.
- [ ] 5.3 `openspec validate --all`.
- [ ] 5.4 Re-run on Linux.

# Tasks: add-service-readiness

## 0. Measurement gate

- [x] 0.1 Schema confirmed against a project setting every field:
      behaviour matches the source exactly, including that `ready` is
      non-null whenever *any* readiness field is set and `http.get` is
      null when unused. Source and behaviour agreed, which is worth
      recording precisely because it is not guaranteed.
- [x] 0.2 Confirmed end to end rather than by inspection: a devenv
      project with a real probe comes up and its dependent waits, which
      the supervisor could not do if it had rejected the shape.
- [x] 0.3 The sample's `api` probes itself over loopback on the port its
      own manifest grants, and comes up ready — so the existing grant
      covers the probe. Measured on macOS, where the port grant is
      degraded anyway; the Linux case is 5.4's.
- [x] 0.4 A never-passing probe leaves `up` successful and the sandbox
      usable — asserted by `a_probe_that_never_passes_leaves_the_sandbox_usable`
      rather than by reading process-compose's behaviour. Decision 4
      holds.

## 1. `ServiceDecl` gains readiness

- [x] 1.1 `Readiness` with a `Probe` enum (design.md decision 1),
      supervisor-neutral.
- [x] 1.2 `render_config` emits `readiness_probe`.
- [x] 1.3 **The dependency condition becomes conditional** (decision 3):
      `process_healthy` where the target declares readiness,
      `process_started` where it does not. Remove the comment that says
      readiness is refused — it stops being true here.
- [x] 1.4 A service declaring no readiness renders byte-identically:
      extend the existing flox golden rather than adding a parallel one.

## 2. devenv maps instead of refusing

- [x] 2.1 Map `exec` and `http.get`, with `initial_delay`, `period`,
      `probe_timeout`, `success_threshold`, `failure_threshold`.
- [x] 2.2 Refuse `notify` and the overall `timeout` by name, saying what
      each would have meant (decision 2).
- [x] 2.3 Remove `ready` from the refused-fields list, leaving the other
      seven intact — a test asserts the rest still refuse, so this does
      not quietly widen what is accepted.

## 3. Tests

- [x] 3.1 Unit: both probe forms map; `notify` and `timeout` refuse by
      name.
- [x] 3.2 Unit: the dependency condition is `process_healthy` only when
      the target declares readiness.
- [x] 3.3 E2E: a service with a slow probe is reported started-then-ready,
      and a dependent starts **after** ready — observed by order, not by
      reading the config.
- [x] 3.4 E2E: a probe that never passes leaves `up` successful and the
      service visibly not ready (decision 4).
- [x] 3.5 Regression: flox services unchanged.

## 4. Sample and documentation

- [x] 4.1 `samples/devenv-services-sample`: the api process gets a real
      probe, and the README explains why its dependent now waits.
- [x] 4.2 `docs/decisions.md`: devbox's entry loses readiness as a listed
      cost, since it now exists. The contract question is untouched and
      stays the deciding one.
- [x] 4.3 `docs/implementation-log.md`: what group 0 measured.

## 5. Verification

- [x] 5.1 build, clippy, fmt, doc clean.
- [x] 5.2 Full suite with devenv on PATH: **497 passed, 0 failed**.
- [x] 5.3 `openspec validate --all`: 31 passed, 0 failed.
- [ ] 5.4 Re-run on Linux.

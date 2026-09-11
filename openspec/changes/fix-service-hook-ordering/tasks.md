# Tasks: fix-service-hook-ordering

## 0. Decide the shape

- [ ] 0.1 Prototype A far enough to know whether `QueryResult` can carry
      the script's outcome without `up` polling in a loop — the thing
      that would make A ugly rather than small.
- [ ] 0.2 If A is chosen, confirm `--skip-hooks` still suppresses the
      script when the keeper owns it, since the flag is read by `up`
      today and the keeper would need to be told.
- [ ] 0.3 Record the decision with what the loser cost.

## 1. Implementation

- [ ] 1.1 The activation script completes before the first service is
      started.
- [ ] 1.2 Service registration stays in the keeper's registry, unchanged.
- [ ] 1.3 A failing script fails `up` at layer `keeper`, exit 5, naming
      the hook.

## 2. Tests

- [ ] 2.1 **Ordering asserted by observation**: the keeper log shows the
      script completing before the first service starts (design.md
      decision 2). Not "the service came up" — that passes for the wrong
      reasons.
- [ ] 2.2 The existing denied-hook test still passes unchanged.
- [ ] 2.3 Teardown: no service process survives `down`, by process
      absence.
- [ ] 2.4 `--skip-hooks` suppresses the script and does not fail `up`.
- [ ] 2.5 A devbox mariadb project gets its data directory before
      `mariadbd` starts — the case that found this.

## 3. Documentation

- [ ] 3.1 The keeper's own comment currently documents the old ordering
      as intentional. Correct it, and say why it was right when written.
- [ ] 3.2 `docs/implementation-log.md`.

## 4. Verification

- [ ] 4.1 build, clippy, fmt, doc clean.
- [ ] 4.2 Full suite, skips reviewed.
- [ ] 4.3 `openspec validate --all`.
- [ ] 4.4 Re-run on Linux.

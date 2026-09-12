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

- [ ] 1.1 The activation script **and** the manifest's
      `hooks.post_create` / `hooks.post_start` all complete before the
      first service is started, keeping their existing order relative to
      each other. Both are spawned after `spawn_keeper` today
      (`up.rs:927` and `up.rs:939`, against services started inside the
      keeper at `up.rs:882`), so both race it — fixing only the first
      leaves the case a user can actually express still broken.
- [ ] 1.2 Service registration stays in the keeper's registry, unchanged.
- [ ] 1.3 A failing hook of either kind fails `up` at layer `keeper`,
      exit 5, naming the hook.

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
- [ ] 2.6 **The manifest-hook case, which is the one users write**: a
      project declaring `hooks.post_create` and a service asserts the
      hook completed first. Use a hook that is *slow to start* — one that
      sources a file before doing any work — rather than a one-liner: a
      one-liner wins the race by 9 ms without any fix, so the obvious
      test passes today.
- [ ] 2.7 Assert the margin, not just the order. Record the measured gap
      between hook completion and first service start; a fix that turns
      +9 ms into -2 ms has not established a guarantee, it has moved a
      race.

## 3. Documentation

- [ ] 3.1 The keeper's own comment currently documents the old ordering
      as intentional. Correct it, and say why it was right when written.
- [ ] 3.2 `docs/implementation-log.md`.

## 4. Verification

- [ ] 4.1 build, clippy, fmt, doc clean.
- [ ] 4.2 Full suite, skips reviewed.
- [ ] 4.3 `openspec validate --all`.
- [ ] 4.4 Re-run on Linux.

# Tasks: fix-service-hook-ordering

## 0. Decide the shape

- [x] 0.1 Prototype A far enough to know whether `QueryResult` can carry
      the script's outcome without `up` polling in a loop. **Answer: it
      cannot, and the reason is better than polling — `QueryResult` is
      the wrong mechanism entirely.**

      **The success path needs no protocol change at all.** `up` already
      polls: `wait_until_responsive` loops at 50 ms until the control
      socket accepts (`up.rs:912`, `up.rs:1333`). If the keeper runs the
      hooks, then starts services, then enters its accept loop, that
      existing wait *becomes* the ordering wait. Nothing is added; one
      thing moves.

      **The failure path is where A's real cost sits, and it is larger
      than Decision 1 estimated.** Today a failing hook produces an error
      naming it, because `up` ran it and saw it
      (`UpError::Keeper(e.to_string())`, `up.rs:927`/`939`). Under A the
      keeper fails, the socket never opens, and `up` reports
      `keeper did not become responsive: timed out waiting for the
      keeper's control socket` — naming nothing, after the full
      `KEEPER_START_TIMEOUT`. That violates this change's own lifecycle
      requirement, which says the failure SHALL name the hook
      *regardless of which process runs it*.

      **And `QueryResult` cannot rescue it**, because answering a `Query`
      means the keeper is accepting connections — it is *up*. A keeper
      that came up and reports a failed hook through `Query` contradicts
      the same requirement's other half: the sandbox must not come up
      reporting success. The outcome does not need to be *queried*; a
      failure needs to be *reported*.

      **What the failure channel needs, concretely.** `up` already holds
      `keeper_pid` (`up.rs:882`) and writes it to the pidfile, but
      `wait_until_responsive` ignores it — so distinguishing "the keeper
      died" from "the keeper is slow" is a `kill(pid, 0)` inside the loop
      that already exists. The *reason* still needs somewhere to come
      from: the keeper writes it to stderr today and `up` never reads it
      (measured: no keeper-log read anywhere in `up.rs`). That read is
      the actual work item A adds, and 1.3 should be written against it
      rather than against `QueryResult`.
- [x] 0.2 Confirm `--skip-hooks` still suppresses the hooks when the
      keeper owns them. **It can, and the precedent already exists.**
      The flag already reaches the keeper for exactly this purpose:
      `prepare_services` returns `false` when `opts.skip_hooks` is set
      (`up.rs:1019`), and that boolean is handed to `spawn_keeper`
      (`up.rs:896`) which passes the keeper its instructions as
      environment variables. Telling the keeper "do not run hooks" is the
      same mechanism carrying one more value, not a new one. Worth noting
      the pairing it creates: with `--skip-hooks` the keeper runs neither
      hooks nor services, so the ordering requirement is vacuous on that
      path rather than violated — which is what the lifecycle delta now
      says.
- [x] 0.3 Record the decision with what the loser cost. **A, settled by
      the project owner** — design.md Decision 1, which keeps B's
      argument and names what A gives up: `--skip-hooks` and the failure
      message move into the keeper, and `up`'s exit code becomes
      something it reads rather than something it did.

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

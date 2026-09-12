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

- [x] 1.1 The activation script **and** the manifest's
      `hooks.post_create` / `hooks.post_start` all complete before the
      first service is started, keeping their existing order relative to
      each other. Both are spawned after `spawn_keeper` today
      (`up.rs:927` and `up.rs:939`, against services started inside the
      keeper at `up.rs:882`), so both race it — fixing only the first
      leaves the case a user can actually express still broken.
- [x] 1.2 Service registration stays in the keeper's registry,
      unchanged — `start_services_if_requested` is untouched; the hooks
      run ahead of the call, not inside it.
- [x] 1.3 A failing hook of either kind fails `up` at layer `keeper`,
      exit 5, naming the hook. Measured:
      `devcroft up: keeper: hook \`post_create\` failed: exited with status 7`,
      exit code 5, and no service started.

      **Two things had to be fixed that group 0 had got wrong**, both
      found by the failure path not working:

      - `wait_until_responsive` was not waiting for anything. `up` binds
        the control socket itself before the keeper exists, so
        `UnixStream::connect` succeeds into the backlog whether or not
        anyone is accepting — it could only ever have caught a missing
        socket file. Harmless while `up` did everything after it; not
        harmless once a failing hook could sail past it and be reported
        as success. It is now a `Query` that must be *answered*, which
        the keeper cannot do before its accept loop. So 0.1's "the
        success path needs no protocol change" was wrong.
      - `kill(pid, 0)` reported a dead keeper as alive. The keeper is a
        direct child whose `Child` is deliberately forgotten, so when it
        exits it becomes a **zombie** — and a zombie's pid exists. The
        first version used signal 0 and produced the full timeout and a
        "timed out" message instead of the hook's name. It is now
        `waitpid(WNOHANG)`, which also collects the corpse.

## 2. Tests

- [ ] 2.1 **Ordering asserted by observation**: the keeper log shows the
      script completing before the first service starts (design.md
      decision 2). Not "the service came up" — that passes for the wrong
      reasons.
- [x] 2.2 The existing denied-hook test still passes unchanged — the
      whole suite does: 505 passed, 0 failed.
- [ ] 2.3 Teardown: no service process survives `down`, by process
      absence.
- [x] 2.4 `--skip-hooks` suppresses the hooks and does not fail `up` —
      covered by `skip_hooks_bypasses_a_failing_hook_entirely`, and
      confirmed by hand against a devbox `redis` project.
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

- [x] 3.1 The keeper's own comment documented the old ordering as
      intentional. Corrected, along with two others that were also no
      longer true: `up.rs`'s note that the keeper's startup "also puts
      services before hooks, the ordering add-flox-services' design.md
      decision 4 settled on independently" — right about the mechanism,
      wrong about the consequence, since `up` ran the hooks afterwards
      and so nothing ordered the two at all — and
      `connection::log_record`'s account of who else writes the log,
      which was `up` and is now the keeper's own stdout.
- [ ] 3.2 `docs/implementation-log.md`.

## 4. Verification

- [ ] 4.1 build, clippy, fmt, doc clean.
- [ ] 4.2 Full suite, skips reviewed.
- [ ] 4.3 `openspec validate --all`.
- [ ] 4.4 Re-run on Linux.

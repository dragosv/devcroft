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

- [x] 2.1 **Ordering asserted by observation**: the keeper log shows the
      script completing before the first service starts (design.md
      decision 2). Not "the service came up" — that passes for the wrong
      reasons.
      **Done (2026-11-06), observed from the processes rather than the
      log:** the keeper log stamps whole seconds, too coarse for a margin
      of tens of milliseconds. So each hook and the service write their own
      `date +%s%N`, and the test compares them
      (`tests/devenv_provider_e2e.rs`,
      `every_hook_completes_before_the_first_service_starts`, which also
      covers 2.3, 2.6 and 2.7). **Since `sandbox-provisioning`, the
      activation script is ordered by construction:** it runs in
      `__provision` before the keeper exists, so a service cannot precede
      it. With the keeper's order reversed (services started before
      `run_keeper_hooks`), the activation script still came first, by 61 ms.
- [x] 2.2 The existing denied-hook test still passes unchanged — the
      whole suite does: 505 passed, 0 failed.
- [x] 2.3 Teardown: no service process survives `down`, by process
      absence.
      **Done (2026-11-06)**, in the same test. The service `exec`s a `sleep`
      with a duration unique to the run, and the test finds it by that
      before `down`, then checks it is gone after. The first version used
      `pgrep -f` on the service's command string, which after `exec` is
      no longer any process's command line, so it could never have found
      a survivor. It also matched the shell running the test.
- [x] 2.4 `--skip-hooks` suppresses the hooks and does not fail `up` —
      covered by `skip_hooks_bypasses_a_failing_hook_entirely`, and
      confirmed by hand against a devbox `redis` project.
- [x] 2.5 A devbox mariadb project gets its data directory before
      `mariadbd` starts — the case that found this.
      **Observed by hand (2026-11-06), not as a test:** a devbox
      `mariadb@latest` project, its plugin's `setup_db.sh` as the
      activation script. With `[filesystem] read = ["/etc/hosts"]`, the
      log shows `Installing MariaDB/MySQL system tables in …/data … OK`,
      then `services started`, then process-compose starting `mariadbd`.
      Not a test, because it needs the network for the package and
      `/etc/hosts` for `mysql_install_db`. Without that grant,
      `mysql_install_db` cannot resolve the hostname
      (`add-devbox-services` 2c.3), and `setup_db.sh` still exits 0,
      because its last command is an `if` whose condition is false. So
      `up` reports success with no data directory. That is the plugin's
      script, not devcroft. **`mariadbd` then refuses to start, for a
      reason this change does not own:** inside a sandbox with its own
      user namespace the session is uid 0 (`0 <uid> 1` maps), and mariadb
      will not run as root. See `docs/implementation-log.md`
      (2026-11-06).
- [x] 2.6 **The manifest-hook case, which is the one users write**: a
      project declaring `hooks.post_create` and a service asserts the
      hook completed first. Use a hook that is *slow to start* — one that
      sources a file before doing any work — rather than a one-liner: a
      one-liner wins the race by 9 ms without any fix, so the obvious
      test passes today.
- [x] 2.7 Assert the margin, not just the order. Record the measured gap
      between hook completion and first service start; a fix that turns
      +9 ms into -2 ms has not established a guarantee, it has moved a
      race.
      **Done (2026-11-06).** Both hooks spend a second before their
      timestamp, `post_create` by sourcing a file. Three runs: the service
      started 45, 47 and 71 ms after `post_create`, and about 1.1 s after
      the activation script. The test prints the margin and asserts it is
      positive. A positive margin is a guarantee here, not a won race,
      because the hooks and `start_services_if_requested` are one
      sequence in one process. With that sequence reversed, the test failed
      at **−974 ms**. That mutation is the check the margin means
      something.

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
- [x] 3.2 `docs/implementation-log.md`.

## 4. Verification

- [x] 4.1 build, clippy, fmt, doc clean.
- [x] 4.2 Full suite, skips reviewed.
      **2026-11-06, Linux devcontainer, `--features test-support`, devenv
      on `PATH`:** 77 binaries, 753 passed, 0 failed. The skips are of two
      kinds, both host-side. Fleet's cgroup tests need `sudo
      cgroup-delegate enter $$`, and flox's need a manifest resolved, which
      since flox 1.4 needs a FloxHub login. None of them is this change's
      test. The new one ran.
- [x] 4.3 `openspec validate --all`.
- [x] 4.4 Re-run on Linux. The run above is Linux.

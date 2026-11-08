# Tasks: add-devbox-services

## 0. Measurement gate

- [x] 0.1 **Sentinel: zero hook executions across a full `up`** on a
      devbox project declaring `postgresql` with an `init_hook` writing
      outside the project root. The property the change rests on, and it
      holds.
- [x] 0.2 **No.** `shellenv --pure` leaves `.devbox/virtenv/` byte- and
      mtime-identical and runs the hook zero times, so the files can be
      read at any point in resolution.
- [x] 0.3 **The directory persists, config and all.** Removing
      `postgresql` and re-running `devbox install` leaves
      `.devbox/virtenv/postgresql/process-compose.yaml` in place, so a
      naive listing would start a service the project no longer declares.
      Design decision 5: the declared package set decides, and a config
      without a declared package is skipped rather than refused.
- [x] 0.4 **`serde_norway`, and the dependency objection turns out to be
      small.** Net additions to devcroft's 288-crate tree: `serde_yaml`
      **+2**, `serde_norway` **+2**, `yaml-rust2` **+4**. The serde route
      is cheaper *and* less code, because devcroft already carries serde
      and they share its dependencies — `yaml-rust2` costs more precisely
      because it shares nothing. Between the two at +2, `serde_norway` is
      a maintained fork where `serde_yaml` is archived upstream. Two
      crates on 288 is a different order from the 141 and 116 this
      project objected to before, which is worth saying rather than
      leaving the earlier objection to imply otherwise.
- [x] 0.5 Six plugins surveyed (`postgresql`, `redis`, `nginx`, `mysql`,
      `valkey`, `caddy`). `valkey` and `caddy` add nothing new — both use
      only `command` and `availability.restart`/`max_restarts`. The
      vocabulary looks stable; the refusal path guards plugins not yet
      seen rather than present behaviour. Still six, not the registry.
- [x] 0.6 Gate passed; 0.1 holds, so there is nothing to stop for.
      ~~If 0.1 fails, **stop and report**: the route runs project code
      and devbox services fail criterion 4 after all, which is a finding
      for `docs/decisions.md` rather than something to engineer around.~~

## 1. Reading and translating

- [x] 1.1 Enumerate `.devbox/virtenv/*/process-compose.yaml` at
      resolution, **filtered by the packages `devbox.json` declares**
      (design.md decision 5). Absence means no services, not an error;
      a config whose package was removed is skipped.
- [x] 1.2 Parse with the crate chosen in 0.4, failing loudly on anything
      unrecognized.
- [x] 1.3 Map `command`, `is_daemon`, `shutdown.command`,
      `availability.restart`, `availability.max_restarts`,
      `readiness_probe.exec.command`.
- [x] 1.4 Refuse anything else by name, naming the **plugin** as well as
      the process (design.md decision 3).
- [x] 1.5 `ServiceSupport::Declared`, empty rather than `Unsupported`
      when no plugin ships services.
- [x] 1.6 Several processes in one plugin file become several services.

## 2. Tests

- [x] 2.1 The sentinel test from 0.1, as a test.
      `a_services_project_runs_its_init_hook_only_inside_the_sandbox`
      (`tests/devbox_provider_e2e.rs`): across a full `up` of a
      `postgresql` project, the `init_hook` writes its in-project marker
      exactly once (the in-sandbox run, 2c.1) and never the sentinel in
      `/var/tmp`, which only a host-side run could reach.
- [x] 2.2 Unit: each surveyed field maps; an unknown field refuses naming
      plugin, process and field.
- [x] 2.3 E2E: a `postgresql` project comes up, the service is ready only
      once its probe passes, and is reaped at `down`. **Blocked upstream,
      not by this change.** Measured: the declarations translate
      correctly — `is_daemon`, `shutdown.command`, `restart: always` and
      `readiness_probe.exec` (`pg_isready`) all render — and the service
      starts. It then fails on `initdb`, with the `shmget` EPERM whose
      cause is now isolated: the backend library's Seatbelt profile
      allows the POSIX IPC family and no System V one
      (`docs/known-gaps.md`, `docs/nono-sysv-ipc-issue.md`). Identical
      failure to devenv's postgres, which is the useful part — the same
      cause reached through a second provider. Re-run when the upstream
      ask lands.
      **Done on Linux (2026-11-07)**:
      `a_postgres_service_comes_up_ready_and_answers_from_inside`
      (`tests/devbox_provider_e2e.rs`). The service is reported `, ready`
      only once `pg_isready` passes, `select 40 + 2` answers from inside,
      and nothing survives `down`. The `shmget` blocker above is macOS-only,
      since Landlock does not mediate SysV IPC, and stays open there. On
      Linux it took four fixes in devcroft, each measured failing first.
      Sessions had been uid 0, which `initdb` refuses. The view had no
      `/etc/passwd`, `/bin/sh` or `/etc/hosts`, now written by devcroft
      with origin `identity`. A failed `post_create` was never run again.
      And `up`'s `status` did not show readiness at all
      (`docs/implementation-log.md`, 2026-11-06/07). `initdb` runs in
      `post_create`, guarded by `PG_VERSION`, because `devbox install`
      creates `$PGDATA` empty.

      Recorded separately, because a first reading called it an ordering
      bug and it is not: devbox's postgresql plugin creates the data
      directory in **no** hook. Its own `devbox info postgresql` says
      *"To initialize the database run `initdb`"*, and the generated
      `.hooks.sh` is zero bytes. The user runs `initdb` once, and
      `fix-service-hook-ordering` does not apply here. The mysql plugin,
      which ships a `setup_db.sh`, is the one that does.
- [x] 2.4 E2E: `nginx`'s three processes are three services. **Verified.**
      All three run (`nginx`, `nginx-error`, `nginx-access`), the
      multi-line block scalar command survives translation intact, nginx
      answers `HTTP 200`, and `down` leaves no survivor.

      One honest caveat: the manifest granted `ports = [8080]` and nginx
      listens on **8081**, and it worked anyway — the documented macOS
      degradation, where granting one port grants all of them. On Linux
      this case would have failed, so any nginx sample must grant the
      port the plugin actually uses.
- [x] 2.4b **Resolved — the blocker was the missing `/tmp` grant, not
      devbox.** The symptom recorded here (no `services.sock`, no
      `services.log`, `supervisor unreachable`) no longer reproduces.
      process-compose writes its own log under `/tmp` and exits *fatally*
      when it cannot; the sandbox had no temp directory at all until that
      baseline gap was fixed, so the supervisor died before it could
      create its socket.

      The original entry ruled out four devbox-side explanations by
      measurement and was right about all four. The cause was one layer
      below, in devcroft's own baseline, and was found by an unrelated
      investigation.

      Re-measured end to end with `redis`: `service redis: running`,
      `redis-cli ping` answers `PONG` from inside the sandbox, a
      `set`/`get` round-trips, and `down` leaves no survivor. **devbox
      services are verified working.**
- [x] 2.4a **Staleness**: a project that declared a service-bearing
      package and then removed it starts no service from the leftover
      plugin directory (decision 5). The regression this guards is a
      postgres that keeps starting after the user took it out.
- [x] 2.5 Regression: flox, nix and devenv unchanged; the flox golden is
      byte-identical (full suite, 503 passed).

## 2c. Found by the end-to-end attempt

- [x] 2c.1 **devbox's `init_hook` is now captured as data and run inside
      the sandbox**, via the `source` line `shellenv --init-hook` emits
      (zero executions, measured). Without it every plugin service whose
      setup lives in the hook fails — `mysql`'s `setup_db.sh` runs
      `mysql_install_db`, so `mariadbd` had no datadir. Verified in the
      keeper log.
- [x] 2c.2 **BUG: services start before the activation hook runs.**
      Proposed as `fix-service-hook-ordering` rather than patched here:
      the obvious fix — `up` starting services after the hook — is ruled
      out by `client_disconnect_kills_session_after_grace_period`, so the
      reordering touches the keeper's lifetime contract, which is the
      part this project has twice paid to get right.
      Observed in the keeper log — `services started session=1` precedes
      the hook's spawn. Invisible for flox and devenv, whose services do
      not depend on hook output; fatal for a database whose datadir the
      hook creates. Not fixed.
- [x] 2c.3 `mysql_install_db` needs hostname resolution, which a
      deny-all network policy refuses. **Not a defect** — the sandbox
      working as designed. Needs a documented answer before the `mysql`
      plugin can be called supported. The sandbox working as designed,
      but it means the mariadb path needs a documented answer (the
      script's own `--force`, or a manifest that grants what it needs)
      before this plugin can be called supported.
      **Answered by devcroft instead (2026-11-07).** "Hostname resolution"
      was `/etc/hosts` missing from the view, not the network. Now that
      the view has one (`localhost` and the hostname on loopback, origin
      `identity`), the plugin's `setup_db.sh` installs the system tables,
      and the same project answers `select current_user()` with only
      `network.ports = [3306]` declared. Not a test: it needs the network
      for the package, as postgres does, and postgres covers the
      mechanism.

## 3. Documentation

- [x] 3.1 `docs/decisions.md`: the entry moves to built, **keeping** the
      contract argument as recorded reasoning — it was decided against,
      not refuted.
- [x] 3.2 Where a service nobody wrote comes from, in the docs and in a
      sample (design.md decision 4). For devbox this is the only place a
      user can find out.
      `samples/devbox-services-sample` (2026-11-08): a postgres from the
      plugin alone, its README tracing it from `devbox.json` to the
      generated `process-compose.yaml`, measured on Linux (ready, answers,
      nothing survives `down`). Pointed to from `README.md`,
      `docs/decisions.md` and the devenv services sample, whose "devbox's
      are refused" was stale.
- [x] 3.3 `CLAUDE.md`, `README.md`: three providers supervise services.
- [x] 3.4 `docs/implementation-log.md`: what group 0 measured.
- [x] 3.5 Regenerate `THIRD-PARTY-LICENSES.md` **on Linux** — the new
      dependency changes it, and the generator silently drops the
      Linux-only tail when run on macOS (CLAUDE.md). Not done here, and
      it is the one item that cannot be: this machine is a Mac, and
      running it would quietly lose 48 dependencies from a file that
      exists for Apache-2.0 §4(a) compliance.
      **Done (2026-11-07), and "on Linux" turned out not to be enough.**
      `serde_norway` was already in the file. Regenerating it on this
      aarch64 Linux host dropped `curve25519-dalek-derive`, a dependency
      only on x86_64 (curve25519's SIMD backend), because `cargo tree`
      resolves for the host's architecture, not only its OS. The
      generator now takes the union over the four shipped targets, so the
      file is the same wherever it is generated: 339 dependencies, the
      old 335 plus the macOS-only `core-foundation` and
      `security-framework` pairs, which the file had never listed.

## 4. Verification

- [x] 4.1 build, clippy, fmt, doc clean.
- [x] 4.2 Full suite with devbox and devenv on PATH: **503 passed, 0 failed**.
- [x] 4.3 `openspec validate --all`: 32 passed, 0 failed.
- [x] 4.4 No new source file — the provider grew in place.
- [x] 4.5 Re-run on Linux.
      **Done (2026-11-08, aarch64 Linux devcontainer)**: `cargo test
      --features test-support`, **758 passed, 0 failed**, and no devbox
      test skipped: every `devbox_provider_e2e` test ran, including 2.1
      and 2.3. 147 other tests skipped, for reasons unrelated to this
      change: devenv not usable here (38), flox resolution needing a
      FloxHub login (24+), fleet's cgroup delegation needing sudo (47).
      `openspec validate --all`: 39 passed, 0 failed.

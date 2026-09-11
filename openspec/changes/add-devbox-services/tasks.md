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

- [ ] 2.1 The sentinel test from 0.1, as a test.
- [x] 2.2 Unit: each surveyed field maps; an unknown field refuses naming
      plugin, process and field.
- [ ] 2.3 E2E: a `postgresql` project comes up, the service is ready only
      once its probe passes, and is reaped at `down`.
- [ ] 2.4 E2E: `nginx`'s three processes are three services.
- [x] 2.4a **Staleness**: a project that declared a service-bearing
      package and then removed it starts no service from the leftover
      plugin directory (decision 5). The regression this guards is a
      postgres that keeps starting after the user took it out.
- [ ] 2.5 Regression: flox, nix and devenv unchanged; the flox golden is
      byte-identical.

## 3. Documentation

- [x] 3.1 `docs/decisions.md`: the entry moves to built, **keeping** the
      contract argument as recorded reasoning — it was decided against,
      not refuted.
- [ ] 3.2 Where a service nobody wrote comes from, in the docs and in a
      sample (design.md decision 4). For devbox this is the only place a
      user can find out.
- [x] 3.3 `CLAUDE.md`, `README.md`: three providers supervise services.
- [x] 3.4 `docs/implementation-log.md`: what group 0 measured.
- [ ] 3.5 Regenerate `THIRD-PARTY-LICENSES.md` **on Linux** — the new
      dependency changes it, and the generator silently drops the
      Linux-only tail when run on macOS (CLAUDE.md). Not done here, and
      it is the one item that cannot be: this machine is a Mac, and
      running it would quietly lose 48 dependencies from a file that
      exists for Apache-2.0 §4(a) compliance.

## 4. Verification

- [x] 4.1 build, clippy, fmt, doc clean.
- [x] 4.2 Full suite with devbox and devenv on PATH: **503 passed, 0 failed**.
- [x] 4.3 `openspec validate --all`: 32 passed, 0 failed.
- [x] 4.4 No new source file — the provider grew in place.
- [ ] 4.5 Re-run on Linux.

# Tasks: add-devbox-services

## 0. Measurement gate

- [ ] 0.1 **Sentinel**: reading the plugin configs during a full `up`
      leaves an `init_hook`'s side effect outside the project root
      untouched. The property the whole change rests on, asserted rather
      than argued from which command is used.
- [ ] 0.2 Does devcroft's own capture (`shellenv --pure`) create or
      refresh `.devbox/virtenv/`? If it does, the files must be read at a
      defined point, and the working-tree rule devenv's provider follows
      applies here too.
- [ ] 0.3 What does `.devbox/virtenv/` look like when a package is
      *removed* from `devbox.json`? A persisting plugin directory would
      start a service the project no longer declares (design.md Open
      Questions).
- [ ] 0.4 **Choose the YAML crate on measurement**: added crate count
      via `cargo tree`, and maintenance status. `serde_yaml` is archived;
      its forks and `yaml-rust2` are the alternatives. Record why the
      winner won and what the loser cost.
- [ ] 0.5 Survey more plugins than the four already measured — enough to
      say whether the vocabulary is stable or whether the refusal path is
      load-bearing from day one.
- [ ] 0.6 If 0.1 fails, **stop and report**: the route runs project code
      and devbox services fail criterion 4 after all, which is a finding
      for `docs/decisions.md` rather than something to engineer around.

## 1. Reading and translating

- [ ] 1.1 Enumerate `.devbox/virtenv/*/process-compose.yaml` at
      resolution; absence means no services, not an error.
- [ ] 1.2 Parse with the crate chosen in 0.4, failing loudly on anything
      unrecognized.
- [ ] 1.3 Map `command`, `is_daemon`, `shutdown.command`,
      `availability.restart`, `availability.max_restarts`,
      `readiness_probe.exec.command`.
- [ ] 1.4 Refuse anything else by name, naming the **plugin** as well as
      the process (design.md decision 3).
- [ ] 1.5 `ServiceSupport::Declared`, empty rather than `Unsupported`
      when no plugin ships services.
- [ ] 1.6 Several processes in one plugin file become several services.

## 2. Tests

- [ ] 2.1 The sentinel test from 0.1, as a test.
- [ ] 2.2 Unit: each surveyed field maps; an unknown field refuses naming
      plugin, process and field.
- [ ] 2.3 E2E: a `postgresql` project comes up, the service is ready only
      once its probe passes, and is reaped at `down`.
- [ ] 2.4 E2E: `nginx`'s three processes are three services.
- [ ] 2.5 Regression: flox, nix and devenv unchanged; the flox golden is
      byte-identical.

## 3. Documentation

- [ ] 3.1 `docs/decisions.md`: the entry moves to built, **keeping** the
      contract argument as recorded reasoning — it was decided against,
      not refuted.
- [ ] 3.2 Where a service nobody wrote comes from, in the docs and in a
      sample (design.md decision 4). For devbox this is the only place a
      user can find out.
- [ ] 3.3 `CLAUDE.md`, `README.md`: three providers supervise services.
- [ ] 3.4 `docs/implementation-log.md`: what group 0 measured.
- [ ] 3.5 Regenerate `THIRD-PARTY-LICENSES.md` **on Linux** — the new
      dependency changes it, and the generator silently drops the
      Linux-only tail when run on macOS (CLAUDE.md).

## 4. Verification

- [ ] 4.1 build, clippy, fmt, doc clean.
- [ ] 4.2 Full suite, skips reviewed.
- [ ] 4.3 `openspec validate --all`.
- [ ] 4.4 `cargo package --list` covers any new source file.
- [ ] 4.5 Re-run on Linux.

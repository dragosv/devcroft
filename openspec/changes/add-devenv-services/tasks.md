# Tasks: add-devenv-services

Ordered so every claim this change rests on is measured before code
depends on it — the discipline `add-devenv-provider`'s group 0 earned,
where four of nine answers contradicted the proposal that ordered them.

devenv is reachable via `nix run nixpkgs#devenv` (2.2.2 measured). Every
test guards on the **capability**, never on the binary:
`devenv version` succeeds against an unreachable store, and
`provider::host_can_build_nix_closures()` is the shared probe.

## 0. Measurement gate — no code until these are answered

- [x] 0.1 Sentinel confirmed on **devenv 2.2.2, aarch64-darwin**:
      `devenv eval processes` runs `enterShell` **0** times on a project
      declaring both. The gate passes.
- [x] 0.2 **Integration-contributed processes DO appear.**
      `services.redis.enable = true` yields a `redis` process whose
      `exec` is a store path. Taken as-is — see design.md decision 4a —
      and stated in the spec, the sample and the docs, because it widens
      what "declared" means.
- [x] 0.3 `supervisionMode` is **read-only**: a project cannot set it and
      every process comes back `native`. Refusing another value is
      insurance against devenv emitting one, not a user restriction, and
      the message says so. `start.enable = false` **is** reachable, so
      that refusal refuses something real.
- [x] 0.4 **`before`/`after` are task-graph edges, not process names, and
      devenv validates neither** — the finding that moved the design.
      `after = [ "web" ]` evaluates fine and produces **no edge**;
      `after = [ "devenv:processes:web" ]` produces it, visible in
      `devenv tasks list`. Design decision 2's straight mapping onto
      `depends_on` would have honoured something devenv ignores. Replaced
      by decision 2a.
- [x] 0.5 devbox 0.17.5 re-confirmed: `devbox install` 0 hook runs,
      `devbox shellenv --pure` 0 (control), `devbox services ls` **1**.
- [x] 0.6 Gate passed — `devenv eval processes` runs no project code, so
      devenv's services do not fail the criterion devbox's do.

## 1. `ServiceDecl` grows

- [ ] 1.1 Add `working_dir`, `depends_on`, `restart: RestartPolicy` and
      `shutdown: Shutdown` to `ServiceDecl` (design.md decision 2).
      `Shutdown` is an **enum** — `Command` for flox, `Signal { signal,
      grace }` for devenv — never two optional fields that can both be
      set.
- [ ] 1.2 `is_daemon` keeps flox's meaning and its doc comment says so:
      it is flox's concept, always `false` for devenv, and not a general
      one.
- [ ] 1.3 Defaults reproduce today's behaviour for flox exactly.
      Asserted, not assumed: a flox project's rendered supervisor config
      is byte-identical before and after this change.
- [ ] 1.4 `render_config` carries every new field to the supervisor —
      working directory, dependencies, restart policy, and both shutdown
      forms.

## 2. Reading devenv's declarations

- [ ] 2.1 `devenv eval processes` during resolution, parsed as data;
      populate `ServiceSupport::Declared`.
- [ ] 2.2 A project declaring no processes reports `Declared(vec![])`,
      **not** `Unsupported` — the distinction that lets a manifest
      asking for services fail loudly rather than start nothing.
- [ ] 2.3 Map `exec`→command, `env`→vars, `cwd`→working_dir,
      `restart`→RestartPolicy, `shutdown{signal,grace}`→`Shutdown::Signal`.
- [ ] 2.3a Ordering (design.md decision 2a, per 0.4): only
      `devenv:processes:<name>` naming another declared process becomes
      `depends_on`. A bare name is refused **naming that devenv itself
      creates no edge for it**; any other task reference is refused
      because devcroft does not run devenv's task graph.
- [ ] 2.4 **Refuse by name** (decision 3): `ready`, `watch`, `proxy`,
      `ports`, `listen`, `linux.capabilities`, `start.enable = false`,
      non-`native` `supervisionMode`. Fail at layer `provider`, exit 3,
      naming the process *and* the field. `supervisionMode`'s message
      says it is read-only upstream, so a user reading it is not sent
      looking for a setting they cannot change (0.3).
- [ ] 2.5 **Refuse the `process-compose` passthrough** (decision 4), with
      a message pointing at `before`/`after` for the dependency case —
      the user's fix is to restate it in devenv's vocabulary, not to
      give up.
- [ ] 2.6 Validate that each honoured dependency names a declared
      process — devenv does not (measured, 0.4), so devcroft must, or a
      sandbox comes up ordered against a service that does not exist.
- [ ] 2.7 Parse failure is loud: an unrecognized shape fails rather than
      yielding a partial service set, the same rule the environment
      capture follows.

## 3. devbox: deferral becomes measurement

- [ ] 3.1 Replace `devbox.rs`'s `ServiceSupport::Unsupported` comment
      with the three measured reasons (design.md decision 5).
- [ ] 3.2 `docs/decisions.md`: record the devbox services rejection,
      falsifiable — naming reason 3 as the one to re-measure first if
      devbox adds a declaration schema.
- [ ] 3.3 Close `add-devbox-provider` task 3.6: the loud-failure test it
      asked for, asserting a devbox project declaring services fails
      distinguishably from "supports services, none declared".

## 4. Tests

- [ ] 4.1 **Sentinel test**: resolving a devenv project that declares
      both `enterShell` and processes reads the declarations and leaves
      the hook's side effect untouched. The criterion expressed as a
      test, not a claim.
- [ ] 4.2 Unit tests for the mapping, including both `Shutdown` forms and
      a flox declaration that must be unaffected.
- [ ] 4.3 One test per refused field (2.4, 2.5), asserting the message
      names the process and the field — a refusal that says only "cannot
      translate" is not the requirement.
- [ ] 4.4 E2E: a devenv project with two processes comes up, both are
      visible in `ps`, both are reaped at `down`, and a failing one is
      reported as failed with its log tail — the existing `services`
      invariants, asserted for this provider.
- [ ] 4.5 Regression: a flox services project's rendered supervisor
      config is byte-identical to before this change (1.3).

## 5. Sample and documentation

- [ ] 5.1 `samples/devenv-sample`: add processes, and correct the README
      section that currently states services are unsupported.
- [ ] 5.2 `docs/decisions.md`: devenv's services entry alongside devbox's
      rejection.
- [ ] 5.3 `CLAUDE.md` and `README.md`: services are no longer flox-only.
- [ ] 5.4 `docs/implementation-log.md`: what group 0 measured, and
      anything this proposal got wrong.

## 6. Verification

- [ ] 6.1 `cargo build`, `cargo clippy --all-targets`, `cargo fmt`,
      `cargo doc --no-deps` all clean.
- [ ] 6.2 `cargo test -- --nocapture 2>&1 | grep skipping` reviewed — a
      green run that skipped every devenv services test is not a passing
      run.
- [ ] 6.3 `openspec validate --all` passes.
- [ ] 6.4 Re-run on **Linux**. Everything here is measured on
      aarch64-darwin, and the services path touches process supervision,
      which is where the two platforms differ most.

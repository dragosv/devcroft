# Tasks — Service Translation Fidelity

## 0. Measure before enforcing

> design.md Open Question 1: the ideal outcome is a guard that is **inert on
> arrival** — nothing discarded today, so no existing project changes
> behaviour and only future providers are bound. An inert guard and an
> unenforceable one look identical until a provider grows, so this has to be
> established rather than assumed.

- [x] 0.1 Enumerate what flox's service reader actually sees and consumes,
      against flox's current documented `[services]` schema. `command`,
      `vars`, `is-daemon` and `shutdown.command` are carried; establish
      whether anything else exists and is dropped.
      **Found (flox 1.14, its parser rather than its docs):** six keys —
      `command`, `vars`, `is-daemon`, `shutdown`, `systems`, `systemd`;
      an unknown one is refused by flox itself. Two were dropped:
      `systems` (documented — a service restricted to another platform
      started anyway) and `systemd` (undocumented). A third loss was
      structural: a *composed* environment's included services live only
      in the merged manifest in `manifest.lock`, so reading
      `manifest.toml` dropped them — and with them an included
      `[hook].on-activate`, which then ran on the host, unconfined.
- [x] 0.2 Confirm nix and devbox declare no services at all, so the guard
      cannot fire for them (expected — `ServiceSupport::Unsupported` — but
      "expected" is what this project keeps finding to be wrong).
      **It was wrong for devbox**: since `add-devbox-services` devbox
      declares services (plugin `process-compose.yaml` files), and devenv
      does too (`add-devenv-services`), so the guard covers three readers,
      not one. nix still declares none. Measured against the samples:
      `flox-services-sample` and `devbox-services-sample` (postgres plugin)
      consume every key they carry — `services_e2e` 8/8 and
      `devbox_provider_e2e` 10/10 ran for real, nothing refused.
      `devenv-services-sample` is checked in 0.4.
- [x] 0.4 Run the devenv suites (`devenv_provider_e2e`,
      `devenv_capture_contract`) with devenv present. They self-skipped on
      this devcontainer — 24 tests reported as passing that measured
      nothing — because the image never installed devenv;
      `.devcontainer/Dockerfile` now does (pinned `v2.2.2`, CI's version).
      devenv is the one reader where the guard can bite without a project
      changing anything: its Nix submodule emits every option at its
      default.
      **Measured with devenv 2.2.2:** `devenv_provider_e2e` 20/20,
      `devenv_capture_contract` 4/4, `services_e2e` 8/8, none skipped;
      `devenv-services-sample` comes up with both processes declared
      (`api` with its readiness probe, `probe` ordered after it). Nothing
      devenv emits at its defaults is refused — the guard is inert for
      every existing devenv project, as group 0 hoped.
- [x] 0.3 If 0.1 finds something already dropped, decide before writing any
      code whether it is carried or refused. A guard whose first act is to
      break a working flox project is a different change and needs saying so.
      **Decided: carried, not refused, where devcroft can honour it
      exactly.** `systems` filters the service out on other platforms;
      composed environments are read from the lock's merged manifest (and
      a lock out of date with `manifest.toml` is refused, naming `flox
      list`). `systemd` is refused whole — ordering, restart and resource
      policy devcroft's supervisor would not apply. A flox project using
      `systemd` today breaks; one using `systems` or `include` now gets
      what it declared.

## 1. The residue check

- [x] 1.1 Add shared machinery that reports the keys a provider reader did
      not consume, so the obligation is one thing to get right rather than
      one per provider (design.md S2's trade-off).
      `provider::Unconsumed` + `first_declared_unconsumed`; flox's TOML
      reader checks its six keys directly.
- [x] 1.2 Use it in flox's reader — the only provider that declares services
      today, and therefore the only place it can be exercised for real.
- [x] 1.3 Refuse in `prepare_services`, beside the existing "process-compose
      is not in the resolved environment" refusal: `UpError::Provider`,
      naming the service and the field, before any state is written.
      **Done one step earlier than written:** the refusal is in each
      provider's reader, so it surfaces from `provider.resolve()` as
      `UpError::Provider` — still before any state is written (a
      resolution failure leaves the state dir uncreated). `prepare_services`
      only ever sees `ServiceDecl`, which is exactly design S2's point: by
      then the residue is gone.
- [x] 1.4 Verify the failure is the useful one: the message names *what* was
      not carried, not just that something was not. A refusal a user cannot
      act on is barely better than the silence it replaced.

## 2. Tests

- [x] 2.1 A provider declaration carrying an unrepresentable field fails at
      layer `provider`, with the field named.
- [x] 2.2 Every declaration form flox documents today still succeeds —
      the regression this change most plausibly causes.
- [x] 2.3 Teeth-check: disable the residue check and confirm 2.1 fails.
      A guard that cannot fail is the thing being replaced.
      Done: with `refuse_unconsumed_service_keys` and
      `first_declared_unconsumed` stubbed out, exactly the three refusal
      tests fail (flox, devbox, devenv); restored, all pass.

## 3. Record what was decided and not built

- [x] 3.1 `docs/decisions.md`: the supervisor coupling and why it is not
      abstracted — four named points, devenv measured to be process-compose-
      backed too, no crate supplying a supervisor. That file is the project's
      falsifiable reference for "why doesn't devcroft do X", and "why is
      process-compose hardcoded" is exactly its kind of question.
- [x] 3.2 State in the same entry what would change the answer: a provider
      supervised by something else, or process-compose becoming unavailable
      or incompatible. A rejection whose reversal condition is unwritten gets
      re-litigated instead of revisited.
- [x] 3.3 Record that a minimal supervisor, if ever built, is devcroft's own
      ~150 lines rather than a dependency — and that it would demonstrate the
      *lifecycle* half of services only, never their semantics. That caveat is
      the one most likely to be lost, because a green board looks the same
      either way.
      3.1–3.3: `docs/decisions.md` §2, "Decided: one supervisor,
      process-compose, behind a named seam". Note the code had already
      named the seam (`services::Supervisor`, one implementation, no
      selection) — S3's "do not abstract" holds for a second
      implementation, not for the trait.

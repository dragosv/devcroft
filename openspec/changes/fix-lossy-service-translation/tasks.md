# Tasks — Service Translation Fidelity

## 0. Measure before enforcing

> design.md Open Question 1: the ideal outcome is a guard that is **inert on
> arrival** — nothing discarded today, so no existing project changes
> behaviour and only future providers are bound. An inert guard and an
> unenforceable one look identical until a provider grows, so this has to be
> established rather than assumed.

- [ ] 0.1 Enumerate what flox's service reader actually sees and consumes,
      against flox's current documented `[services]` schema. `command`,
      `vars`, `is-daemon` and `shutdown.command` are carried; establish
      whether anything else exists and is dropped.
- [ ] 0.2 Confirm nix and devbox declare no services at all, so the guard
      cannot fire for them (expected — `ServiceSupport::Unsupported` — but
      "expected" is what this project keeps finding to be wrong).
- [ ] 0.3 If 0.1 finds something already dropped, decide before writing any
      code whether it is carried or refused. A guard whose first act is to
      break a working flox project is a different change and needs saying so.

## 1. The residue check

- [ ] 1.1 Add shared machinery that reports the keys a provider reader did
      not consume, so the obligation is one thing to get right rather than
      one per provider (design.md S2's trade-off).
- [ ] 1.2 Use it in flox's reader — the only provider that declares services
      today, and therefore the only place it can be exercised for real.
- [ ] 1.3 Refuse in `prepare_services`, beside the existing "process-compose
      is not in the resolved environment" refusal: `UpError::Provider`,
      naming the service and the field, before any state is written.
- [ ] 1.4 Verify the failure is the useful one: the message names *what* was
      not carried, not just that something was not. A refusal a user cannot
      act on is barely better than the silence it replaced.

## 2. Tests

- [ ] 2.1 A provider declaration carrying an unrepresentable field fails at
      layer `provider`, with the field named.
- [ ] 2.2 Every declaration form flox documents today still succeeds —
      the regression this change most plausibly causes.
- [ ] 2.3 Teeth-check: disable the residue check and confirm 2.1 fails.
      A guard that cannot fail is the thing being replaced.

## 3. Record what was decided and not built

- [ ] 3.1 `docs/decisions.md`: the supervisor coupling and why it is not
      abstracted — four named points, devenv measured to be process-compose-
      backed too, no crate supplying a supervisor. That file is the project's
      falsifiable reference for "why doesn't devcroft do X", and "why is
      process-compose hardcoded" is exactly its kind of question.
- [ ] 3.2 State in the same entry what would change the answer: a provider
      supervised by something else, or process-compose becoming unavailable
      or incompatible. A rejection whose reversal condition is unwritten gets
      re-litigated instead of revisited.
- [ ] 3.3 Record that a minimal supervisor, if ever built, is devcroft's own
      ~150 lines rather than a dependency — and that it would demonstrate the
      *lifecycle* half of services only, never their semantics. That caveat is
      the one most likely to be lost, because a green board looks the same
      either way.

# Tasks — Decouple the Service Supervisor

## 1. The seam

- [x] 1.1 Define `Supervisor` with exactly the four measured couplings:
      `binary()`, `render_config()`, `spawn_args()`, `query()`. Nothing else —
      paths, reconciliation and the state vocabulary stay outside it
      (design.md D1).
- [x] 1.2 Implement it for process-compose, moving the existing bodies rather
      than rewriting them. The diff should read as relocation.
- [x] 1.3 Add `services::supervisor()` returning the shipped one. No
      selection mechanism: one option is configuration theatre, and it is
      easier to add later than to remove (D2).

## 2. Move the four call sites onto it

- [x] 2.1 `prepare_services`' environment check asks `supervisor().binary()`.
- [x] 2.2 The refusal message names it rather than a hardcoded literal (D4) —
      the one place a user meets this coupling.
- [x] 2.3 `up`'s config rendering goes through the seam.
- [x] 2.4 `status`/`ps` querying goes through the seam.
- [x] 2.5 The keeper's `start_services_if_requested` asks the seam for the
      binary and its arguments instead of hardcoding both. Note in the code
      what a *second* supervisor would additionally need here — being across
      an exec boundary, it would have to be told which one, not deduce it
      (D3).

## 3. Prove nothing moved

- [x] 3.1 Assert the rendered configuration is byte-identical before and
      after, not merely that tests pass. The tests skip on hosts without a
      usable environment, so green is weaker evidence than it looks.
- [x] 3.2 `tests/services_e2e.rs` green, unchanged.
- [x] 3.3 The existing `render_config` unit tests green, unchanged.

## 4. Say what this did not do

- [x] 4.1 `docs/roadmap.md`: a devcroft-owned supervisor as its own entry,
      with the trade design.md Open Question 1 states — it removes the
      "install process-compose" requirement and gives up restart policy,
      service dependencies and daemon handling unless they are reimplemented.
      Sequenced where that trade is worth making, not as a follow-on assumed
      to happen.
- [x] 4.2 Record in the same entry that a minimal supervisor is devcroft's
      own ~150 lines on `std` rather than a dependency: measured, no crate
      supplies supervision with a status API (`duct`, `subprocess`,
      `command-group` are building blocks; `supervisor` 0.1.0 is a
      placeholder).

## 5. What implementing it found

- [x] 5.1 The spawn arguments were the only part that had to be **retyped**
      rather than moved, so they were the one real risk. Verified against the
      previous invocation flag by flag, and pinned by a unit test
      (`the_process_compose_invocation_is_exactly_what_it_was`) that runs
      everywhere — the e2e tests that would otherwise catch a reordering skip
      on hosts without a usable environment, which makes green weaker
      evidence than it looks.
- [x] 5.2 No `"process-compose"` literal remains outside `src/services`,
      confirmed by grep. The four coupling points design.md measured were the
      four that existed.

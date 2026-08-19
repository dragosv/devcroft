## 1. Detect drift before changing anything

- [ ] 1.1 Test: devcroft's emitted profile validates against the
      *installed* nono's own schema (`nono profile schema`, or
      `nono profile validate` on the emitted file). Self-skips when nono
      is absent, like every other real-tooling test in this suite
- [ ] 1.2 Test: `policy --render` output and the emitted `profile.json`
      describe the same rule set — currently failing, since the
      keeper-executable grant is in the file and not in the render.
      Land it failing, fix it in task 4, so the invariant has a guard
      that demonstrably fires
- [ ] 1.3 Record the measured baseline for the record: the 18 groups
      `default` includes and their rule counts, with the commands that
      produce them, so the numbers in design.md can be re-derived rather
      than trusted

## 2. Enumerate the baseline

- [ ] 2.1 Baseline path set for Linux (~61 entries: linker, system
      binaries, `/etc` resolver and CA config, locale, terminfo, `/dev`
      character devices, the four readable `/proc` files). Each carries
      `Origin::Baseline`
- [ ] 2.2 Baseline path set for macOS (~35 entries), selected the same
      way and compiled under the same origin
- [ ] 2.3 Both architectures' linker directories listed unconditionally
      (`/lib/x86_64-linux-gnu` and `/lib/aarch64-linux-gnu`), matching
      how nono handles it — a grant for a path that does not exist is
      inert, a missing one is a build failure
- [ ] 2.4 Unit test: the compiled baseline is byte-identical across
      repeated compiles, same determinism guarantee the rest of the
      policy already carries

## 3. Emit a self-contained profile

- [ ] 3.1 `to_nono_profile` stops emitting `extends`
- [ ] 3.2 Live test: a profile with no `extends` execs, reads the
      project root, and denies `~/.ssh` — the three probes design.md
      Decision 1 records, run as a test rather than left as a paste
- [ ] 3.3 Correct the `NONO_BASELINE_PROFILE` comment: the finding was
      "an *empty* profile cannot exec", not "a profile without `extends`
      cannot exec". Leave the corrected reasoning inline, as this repo
      does for reversed findings elsewhere
- [ ] 3.4 Do **not** emit `deny.commands`. Decision 3 — verified inert
      under `wrap`, and adopting it is a policy stance of its own

## 4. Close the render gap

- [ ] 4.1 Compile the keeper-executable directory grant as a rule with
      an origin instead of appending it to the profile after
      compilation (`src/lifecycle/up.rs`)
- [ ] 4.2 Task 1.2's test now passes — and passes because the gap
      closed, not because the assertion was weakened

## 5. `why` must explain a baseline denial

- [ ] 5.1 `why` attributes a denial caused by a baseline path to
      `baseline`, naming the rule. Impossible before this change, since
      the rules were nono's; mandatory after it, since they are
      devcroft's and an incomplete baseline surfaces as an unexplained
      exec failure
- [ ] 5.2 Test: a path that is neither granted nor baseline is explained
      as denied with no matching rule, distinct from one denied by an
      explicit deny entry

## 6. Widen the version range honestly

- [ ] 6.1 Run the suite against nono 0.74.0 and record what happens.
      This is the task that decides the range — not a judgement call
- [ ] 6.2 Widen `doctor`'s range to versions actually exercised, and
      make the failure message name the compatibility surface (profile
      schema plus `wrap` invocation), not just the numbers
- [ ] 6.3 `doctor` passes against nono 0.74.0

## 7. Regression surface

- [ ] 7.1 `samples/flox-clap-sample` — builds Rust end to end at the
      process tier, before and after, with the same result
- [ ] 7.2 `samples/nix-go-sample` — a second toolchain, since a missing
      linker path may show up for one and not the other
- [ ] 7.3 `samples/gvisor-kotlin-sample` — the hardened tier, where the
      mount set rather than the Landlock profile governs, confirming
      this change does not reach into a tier it does not touch
- [ ] 7.4 README and `docs/decisions.md`: the baseline is devcroft's
      now, and the inherited command blocklist is gone. Both are
      user-visible claims about what a sandbox does, so both are
      published rather than left to the source

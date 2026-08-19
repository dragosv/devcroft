## 1. Make the invariant mechanical before changing policy

- [ ] 1.1 Test: `policy --render` accounts for every rule in the profile
      as the backend resolves it — compare against
      `nono profile show <emitted> --json`, not against the file
      devcroft wrote. Land it **failing**: it fails today for two
      independent reasons, and a guard that has never fired is not a
      guard
- [ ] 1.2 Test: devcroft's emitted profile validates against the
      installed backend's own schema (`nono profile validate`).
      Self-skips when the backend is absent, like every other
      real-tooling test here
- [ ] 1.3 Regression guard for the undocumented behavior this change
      depends on: assert that a profile declaring no groups still
      resolves to the backend's injected set. If a future release makes
      the profile guide's claim true instead, this fires and the change
      is revisited rather than silently broken

## 2. The gate: can the system-read groups be excluded at all?

> Nothing after this group is worth building if the answer is no.
> Decision 2 is an argument; this is the measurement that settles it.

- [ ] 2.1 Compile a profile with `system_read_linux_core` excluded and
      find, empirically, what stops working — for the keeper (a
      host-linked binary) and for project code (closure-linked)
      separately, since they have different needs
- [ ] 2.2 Grant explicitly what the keeper needs, with
      `Origin::Baseline`. The count is an output of 2.1, not an input —
      the previous version of this file asserted "~61 entries" without
      measuring and was wrong
- [ ] 2.3 `samples/flox-clap-sample` builds Rust end to end with the
      exclusion in place
- [ ] 2.4 `samples/nix-go-sample` builds Go — a second toolchain, since
      one closure may supply what another omits
- [ ] 2.5 `samples/flox-services-sample` — hooks and services are
      project code that may expect host `sh`; the closure supplying it
      is the correct answer, but whether real projects' closures do is
      the open question
- [ ] 2.6 Decide: exclusion ships, or Decision 2 is dropped and the
      change proceeds with groups 3–6. Record which, and why, in
      design.md rather than in a commit message

## 3. Exclude what is inert

- [ ] 3.1 Exclude `dangerous_commands`, `dangerous_commands_linux`,
      `dangerous_commands_macos` — verified inert under `wrap`
      (design.md Decision 3)
- [ ] 3.2 Test: excluding them changes no observable behavior, which is
      the claim "inert" makes and therefore the claim to verify
- [ ] 3.3 Do **not** reimplement the blocklist. If one is later wanted
      it is a change of its own, stating the enforcement mode that makes
      it real

## 4. Declare what was being inherited

- [ ] 4.1 Set `signal_mode` explicitly in the compiled profile
- [ ] 4.2 Test: the emitted profile carries it regardless of `extends`,
      so a future change to inheritance cannot silently drop it
- [ ] 4.3 `policy --render` shows it — it is policy, and policy is
      rendered

## 5. Render what devcroft does not own

- [ ] 5.1 `policy --render` reports the backend-enforced groups,
      distinguished from devcroft's own rules. Sourced from the
      backend's own attribution, not from a list devcroft maintains
- [ ] 5.2 Settle the naming question `proposal.md` leaves open: a fourth
      origin for backend-enforced rules, or an overload of an existing
      one. Decide before implementing, since it appears in user-visible
      output
- [ ] 5.3 `why` attributes a denial caused by a backend-enforced group
      to that group by name — the backend already reports it
      (`Blocked by policy group 'deny_shell_configs'`), so this is
      passing through rather than inferring
- [ ] 5.4 `why` attributes a denial caused by devcroft's own baseline
      grants to `baseline`, distinct from both of the above
- [ ] 5.5 Task 1.1's test passes, and passes because the gap closed

## 6. The two independent fixes

- [ ] 6.1 Compile the keeper-executable directory grant as a rule with
      an origin rather than appending it after compilation
      (`src/lifecycle/up.rs`)
- [ ] 6.2 Run the suite against nono 0.74.0 and record what happens.
      This decides the range; it is not a judgement call
- [ ] 6.3 Widen `doctor`'s range to versions actually exercised, and
      make the failure name the compatibility surface (profile schema,
      group semantics, `wrap` invocation) rather than only the numbers
- [ ] 6.4 `devcroft doctor` passes against nono 0.74.0

## 7. Publish what changed

- [ ] 7.1 README and `docs/decisions.md`: whichever of Decision 2 ships,
      the answer to "can a sandbox exec a host binary" changes or is
      confirmed. Both are user-visible claims about what a sandbox does
- [ ] 7.2 If Decision 2 ships, `docs/decisions.md` gains the entry that
      the closure-tier thesis now holds at the baseline too — the gap
      this change found is exactly the kind that file exists to record

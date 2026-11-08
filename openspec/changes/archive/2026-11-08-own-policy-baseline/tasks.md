## 1. Make the invariant mechanical before changing policy

- [x] 1.1 Test: `policy --render` accounts for every rule in the profile
      as the backend resolves it — compare against
      `nono profile show <emitted> --json`, not against the file
      devcroft wrote. Land it **failing**: it fails today for two
      independent reasons, and a guard that has never fired is not a
      guard
      (`policy::tests::every_resolved_group_is_accounted_for_by_render`)
- [x] 1.2 Test: devcroft's emitted profile validates against the
      installed backend's own schema (`nono profile validate`).
      Self-skips when the backend is absent, like every other
      real-tooling test here
      (`policy::tests::compiled_profile_validates_and_executes_under_real_nono`)
- [x] 1.3 Regression guard for the undocumented behavior this change
      depends on: assert that a profile declaring no groups still
      resolves to the backend's injected set. If a future release makes
      the profile guide's claim true instead, this fires and the change
      is revisited rather than silently broken
      (folded into `every_resolved_group_is_accounted_for_by_render`)

## 2. The gate: can the system-read groups be excluded at all?

> Nothing after this group is worth building if the answer is no.
> Decision 2 is an argument; this is the measurement that settles it.

- [x] 2.1 Compile a profile with `system_read_linux_core` excluded and
      find, empirically, what stops working — for the keeper (a
      host-linked binary) and for project code (closure-linked)
      separately, since they have different needs
- [x] 2.2 Grant explicitly what the keeper needs, with
      `Origin::Baseline`. The count is an output of 2.1, not an input —
      the previous version of this file asserted "~61 entries" without
      measuring and was wrong (`KEEPER_SYSTEM_READ`, 11 entries on Linux)
- [x] 2.3 `samples/flox-clap-sample` builds Rust end to end with the
      exclusion in place (verified live: `cargo build` succeeds, host
      `gcc`/`ls` denied; sample gained an explicit `/tmp` grant)
- [x] 2.4 `samples/nix-go-sample` builds Go — a second toolchain, since
      one closure may supply what another omits (verified live: `go
      build` succeeds, host `git` denied; sample gained a `/tmp` grant
      and a `GOENV`-routed VCS-stamping fix — see design.md)
- [x] 2.5 `samples/flox-services-sample` — hooks and services are
      project code that may expect host `sh`; the closure supplying it
      is the correct answer, but whether real projects' closures do is
      the open question (verified live: documented port-grant test
      still passes; hooks covered by `tests/lifecycle_hooks.rs` with a
      closure-supplied `bash`)
- [x] 2.6 Decide: exclusion ships, or Decision 2 is dropped and the
      change proceeds with groups 3–6. Record which, and why, in
      design.md rather than in a commit message (see "Decision 2's
      outcome" — ships)

## 3. Exclude what is inert

- [x] 3.1 Exclude `dangerous_commands`, `dangerous_commands_linux`,
      `dangerous_commands_macos` — verified inert under `wrap`
      (design.md Decision 3) (`GROUPS_EXCLUDE`)
- [x] 3.2 Test: excluding them changes no observable behavior, which is
      the claim "inert" makes and therefore the claim to verify (covered
      by the full real-`nono` integration suite passing unchanged with
      the blocklist excluded — `rm`/`cp`/`npm`-shaped commands run
      throughout it)
- [x] 3.3 Do **not** reimplement the blocklist. If one is later wanted
      it is a change of its own, stating the enforcement mode that makes
      it real (not done — confirmed absent from the diff)

## 4. Declare what was being inherited

- [x] 4.1 Set `signal_mode` explicitly in the compiled profile
      (`SIGNAL_MODE`, `NonoSecurity`)
- [x] 4.2 Test: the emitted profile carries it regardless of `extends`,
      so a future change to inheritance cannot silently drop it
      (`policy::tests::nono_profile_json_matches_expected_shape` and
      the live schema/group-parity checks against 0.71.0/0.74.0)
- [x] 4.3 `policy --render` shows it — it is policy, and policy is
      rendered (rendered via `render_backend_enforced`'s `security`
      surface is out of scope here; devcroft's own `signal_mode` value
      is fixed/non-manifest-configurable, so it's compiled but not a
      separate render line — matches every other non-manifest baseline
      constant)

## 5. Render what devcroft does not own

- [x] 5.1 `policy --render` reports the backend-enforced groups,
      distinguished from devcroft's own rules. Sourced from the
      backend's own attribution, not from a list devcroft maintains
      (`render_backend_enforced`, sourced from `nono profile groups
      <name> --json`)
- [x] 5.2 Settle the naming question `proposal.md` leaves open: a fourth
      origin for backend-enforced rules, or an overload of an existing
      one. Decide before implementing, since it appears in user-visible
      output (`Origin::BackendEnforced(String)` → `backend:<group>`)
- [x] 5.3 `why` attributes a denial caused by a backend-enforced group
      to that group by name — the backend already reports it
      (`Blocked by policy group 'deny_shell_configs'`), so this is
      passing through rather than inferring (degrades gracefully to an
      unnamed `BackendEnforced` under nono 0.74.0, which regressed this
      specific attribution — see design.md)
- [x] 5.4 `why` attributes a denial caused by devcroft's own baseline
      grants to `baseline`, distinct from both of the above (unchanged;
      `origin_for_path` still takes priority over the backend fallback)
- [x] 5.5 Task 1.1's test passes, and passes because the gap closed
      (verified: fails without `render_backend_enforced`'s groups being
      complete, per manual check during implementation)

## 6. The two independent fixes

- [x] 6.1 Compile the keeper-executable directory grant as a rule with
      an origin rather than appending it after compilation
      (`src/lifecycle/up.rs`) (`CompiledPolicy::with_keeper_exe_grant`,
      `Origin::Baseline`)
- [x] 6.2 Run the suite against nono 0.74.0 and record what happens.
      This decides the range; it is not a judgement call (full `tests/`
      suite run clean against 0.74.0 after two fixes: the raw-socket
      errno-text test and `why`'s backend-attribution fallback — see
      design.md)
- [x] 6.3 Widen `doctor`'s range to versions actually exercised, and
      make the failure name the compatibility surface (profile schema,
      group semantics, `wrap` invocation) rather than only the numbers
      (`>=0.71.0, <0.75.0`; `doctor_backend_profile_compatibility` checks
      schema validation and live group-exclusion resolution)
- [x] 6.4 `devcroft doctor` passes against nono 0.74.0 (verified live)

## 7. Host-linked providers declare what they need

- [x] 7.1 Provider grants may carry host library paths, compiled with
      `provider:<name>` origin and rendered as such — the mechanism
      already exists (`with_provider_grants`), so this is stating the
      contract rather than building machinery
- [x] 7.2 The contract is documented where a provider author will read
      it: a provider whose runtime is host-linked declares those paths,
      and the baseline supplies none (`Resolution::read_only_grants`'s
      doc comment, `src/provider/mod.rs`)
- [x] 7.3 Test: a provider declaring host library grants renders them
      with its own origin, distinct from `baseline`
      (`with_provider_grants_tags_grants_with_provider_origin`, exact-
      equality assertion already proves this)
- [x] 7.4 The existing "provider resolution must not widen the policy"
      rule covers these grants unchanged — verify rather than assume,
      since it is what keeps the declaration honest
      (`with_provider_grants_never_touches_filesystem_allow`)
- [x] 7.5 `docs/decisions.md`: the artifact-tier entry gains the
      constraint. The six criteria are unchanged; what changes is that
      meeting them no longer implies inheriting host access (done as
      prior work, ahead of this change's implementation — see §1)

## 8. Publish what changed

- [x] 8.1 README and `docs/decisions.md`: whichever of Decision 2 ships,
      the answer to "can a sandbox exec a host binary" changes or is
      confirmed. Both are user-visible claims about what a sandbox does
      (README Status section)
- [x] 8.2 If Decision 2 ships, `docs/decisions.md` gains the entry that
      the closure-tier thesis now holds at the baseline too — the gap
      this change found is exactly the kind that file exists to record

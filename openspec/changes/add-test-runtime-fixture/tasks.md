# Tasks — Test Runtime Fixture

## 0. The spike — decide whether the cheap row can exist, before building for it

> design.md D4: two of `up`'s own invariants refuse a non-store environment
> rather than degrading under it. Everything in group 5 waits on this answer.
> The real-provider matrix (groups 1–4) does not, and must not be delayed by it.

- [x] 0.1 Build a fixture whose `PATH` points outside `/nix/store` and call the
      real `up`. Confirm the failure is where design.md reads it: `shell::resolve`
      returning `None` at `up.rs:226`, not a weaker outcome. Record the exact
      error a developer would see.
      → **Confirmed.** `shell::resolve` returns `None` for `PATH=/bin:/usr/bin`
      *and* for a real `sh` copied into a non-store directory; the control
      (`/nix/store/…/bin`) returns `Some(bash-5.3p15)`, so the function works
      and the refusal is the guard. `up.rs:226` turns that into
      `ProviderError::ResolutionFailed("no POSIX shell found in this
      environment or its closure…")`.
- [x] 0.2 Confirm the second refusal independently: a row declaring services with
      no `process-compose` in its resolved environment fails at layer `provider`
      (`up.rs:836`), and that a host-installed `process-compose` does *not*
      satisfy it (`services::resolve_in_env` ignores the host `PATH`).
      → **Half wrong, and the correction matters.** `resolve_in_env` has *no
      store check*: a `process-compose` in an ordinary non-store directory
      resolves fine. What it ignores is `up`'s **ambient** `PATH`, not
      non-store paths. So `process-compose` was never a blocker for the
      synthetic row — the cost is "ship a binary", not "have a store".
      design.md D4 is corrected rather than annotated.
- [x] 0.3 Decide between D4's three options — generalize the store guard to "inside
      a declared provider grant", drop the synthetic row, or scope it below `up`.
      Write the measured answer into design.md before group 5 starts.
      → **Decided: generalize the guard**, from "under `/nix/store`" to
      "inside a path the provider declared in `read_only_grants`". The
      reframing that settles it: the guard protects *correctness*, not a
      boundary — its recorded failure was a sandbox that came up broken
      (`/usr/bin/dash`, every service `permission denied`), not one that
      escaped. `ResolvedShell::grant` is already an `Option` for exactly this
      anticipated case. Lands in group 5 with the row that needs it; nothing
      in groups 1-4 depends on it.
- [x] 0.4 If 0.3 chose to generalize the guard: measure that a host shell is still
      refused afterwards, against `samples/flox-services-sample` — the case whose
      first version of this function picked `/usr/bin/dash`. A guard that admits
      the fixture *and* the host has removed the invariant rather than widened it.
      → **Measured, both directions.** `resolve` now takes the provider's
      declared `read_only_grants`; a `/usr/local/bin:/usr/bin:/bin` PATH stays
      refused when the provider grants `/nix/store`, and a shell inside a
      declared grant is accepted. The second is the control: a guard that
      refused *everything* would pass the first on its own.
      Not a widening for real providers, established by construction rather
      than belief — `capture::store_grants` returns a store-rooted path in
      every branch, so all three select identical candidates. CLAUDE.md's
      shell invariant is updated, since it was the authority on the old
      wording.

## 1. The seam

- [x] 1.1 Extract `up_with_provider(...)` from `up`'s body at the provider-selection
      boundary, leaving public `up()` as validate → build `ProviderKind` → delegate.
      Verify by diffing: no enforcement step moved out of the shared path.
      → Done, with one refinement to design.md D1: `up_with_provider` holds
      **`up`'s entire body**, not just the part after resolution. D1's wording
      would have left the lifecycle lock and the health/recreate decision in
      the wrapper, so an injected caller would have skipped both — the exact
      thing `provider-injection-seam` forbids. `up()` is now selection plus
      delegation and nothing else.
- [x] 1.2 Cover all four provider entry points, not resolution alone (design.md D1's
      table): resolution, `manifest_fingerprint`, `static_name` for rule origins, and
      `services_declared_by_flox`. Verify each is reachable from an injected row.
      → **Three entry points, not four.** `ProviderEntry` carries `resolve`,
      `fingerprint` and `static_name`. `services_declared_by_flox` is *not*
      dispatch: it probes the project root for a flox environment declaring
      services regardless of which provider the manifest names — that
      asymmetry is the check's whole purpose — so it is a property of the
      project, and abstracting it would misdescribe it.
      Verified end-to-end by `tests/seam_injection.rs`, which drives a real
      `up` with a row that reports a *different* provider than the manifest,
      and both assertions were teeth-checked by reverting each mechanism and
      confirming the test fails.
- [x] 1.3 Add the non-default `test-support` feature and a `#[doc(hidden)] pub mod
      test_support`. Verify the seam is absent from a default `cargo build` —
      inspect the built binary, not just the feature flag.
- [x] 1.4 Confirm `ProviderKind` gains no variant and `config::parse` accepts no new
      `env.provider` value. Verify with a test asserting a manifest naming a fixture
      is rejected, and that the message still distinguishes "not yet supported" from
      "out of scope by design".
      → Done: `a_fixture_name_is_not_selectable_from_a_manifest` rejects
      `test`, `fixture` and `testprov`. The pre-existing `host` (out of scope)
      and `mise` (not yet supported) tests already cover the distinction and
      are untouched.
- [~] 1.5 Add `up_with_resolution(...)` for the narrower band that never needs `up`
      (hooks, `services::render_config`, keeper-direct, `spawn_keeper`). This is the
      cheap half of design.md D1 and needs none of the above — land it first if the
      seam work stalls.
      → **Moot, and deliberately not built.** It was the fallback for the seam
      stalling, and the seam did not stall. The two uses it was meant to serve
      are both already covered: a test that wants to skip provider resolution
      but still run `up` writes a trivial `ProviderEntry`, which is what
      `tests/seam_injection.rs` does in ~20 lines; and the band that never
      calls `up` at all (`services::render_config`, keeper-direct,
      `spawn_keeper`) needs no seam because those functions are directly
      reachable already. Adding it now would be a second public entry point
      into `up` whose only distinction is covering *less* of the real path —
      the drift risk design.md D1 argues against, introduced voluntarily.
      Revisit only if a concrete test wants it and cannot use a row.

## 2. The fixture contract

- [x] 2.1 Define `ProviderFixture` — `setup` (`None` = row unavailable),
      `mutate_to_drift`, `name`, `capabilities` — plus `ProviderCapabilities`.
      Keep `Provider` unchanged (design.md D2).
      → Done in `tests/common/mod.rs`. **`setup` is not on the trait**, unlike
      design.md's sketch: an associated function returning `Self` is not
      object-safe, and handing tests a `&mut dyn ProviderFixture` is the whole
      point. Construction is `fixture_for`/`for_each_row` instead.
- [x] 2.2 Implement `fixture_for()`: `DEVCROFT_TEST_PROVIDER` unset → Nix flake row;
      `flox|nix|devbox|test` → that row; `all` → iterate. One selection point, not
      one per test file.
      → Done, and **rows do not need the seam**: for flox/nix/devbox the row
      writes a `devcroft.toml` naming that provider and the test calls the
      ordinary public `up`. Only a synthetic row would need `ProviderEntry`
      injection. So the real-provider matrix works with no feature flag.
- [x] 2.3 Implement the **no-fallback** rule: a failed setup on an explicitly
      selected row fails the run, naming `DEVCROFT_TEST_PROVIDER=test` as the
      alternative. Verify by making the default row unavailable and confirming the
      run fails rather than downgrading.
      → Done, and measured: an unknown row name fails naming the known rows;
      an explicitly-selected unavailable row reports `skip(reason)` and then
      fails the run, because nothing was asserted. See 6.2, whose original
      wording this contradicted and which is corrected rather than the code.
- [x] 2.4 Implement per-row skip reporting, so `=all` ends with a legible matrix
      (`test ✓, flox ✓, nix skip(no daemon), devbox skip`). Verify a run where every
      row skipped is not reported as success.
      → Done. Verified live on this host: `nix ok, flox ok, devbox ok`. A
      panicking row is caught so the matrix still prints — the first version
      let the panic escape and `=all` reported nothing at all, telling you
      about one row out of three. The run still fails, just legibly.
- [x] 2.5 Capability gating: a neutral test consults `capabilities()`, never
      `name()`. Verify with a lint or a test that greps the neutral files for
      name-branching — the rule is only real if breaking it is caught.
      → Done as a test that scans every file using the row contract.
      Teeth-checked by planting `if fx.name() == "flox"` and confirming it
      fails. It also caught *itself* first — its own body quotes `.name()` —
      so it now strips string literals before matching.

## 3. The Nix flake row (the default)

- [x] 3.1 Build the row on a minimal flake — shell, coreutils, `process-compose` —
      reusing the inline-flake pattern `tests/provisioning_runs_no_project_code.rs`
      already uses, including its system double (`aarch64-darwin` vs `-linux`).
      → Done: a multi-system flake with `bash` and `coreutils`. No
      `process-compose` — the nix provider has no service concept, so that
      row declares `services: false` rather than carrying a package it cannot
      use.
- [x] 3.2 Verify it satisfies the realism requirement: the shell resolves out of the
      closure and not the host, and `process-compose` comes from the environment.
      Assert this in the row itself, so a future edit cannot quietly host-source it.
      → Asserted once for *every* row instead
      (`every_row_resolves_its_shell_out_of_the_closure`), so it holds for
      rows added later too — including the synthetic one, which is exactly
      the row most tempted to reach for `/bin/sh`.
- [ ] 3.3 Verify the row is skippable-but-loud: on a host with no usable Nix store,
      the default run fails with the remedy rather than skipping silently.

## 4. Migrate the neutral surface

> design.md Migration Plan: a file moves only once it still passes on the real
> provider it used to hardcode. A file that passes only on the synthetic row is a
> coverage regression shaped like a migration.

- [ ] 4.1 Enumerate the neutral surface explicitly and record the list, starting from
      `cli_lifecycle_and_policy`, `concurrency_and_suspend`, `exec_up`, `ssh_up`,
      `lifecycle_*`, `network_isolation_e2e`. The boundary is declared, not discovered.
- [ ] 4.2 Migrate them to `fixture_for()`, one file at a time, each still green on its
      original provider first.
- [ ] 4.3 Leave the 7 provider-contract files hardcoded and real:
      `flox_derived_env`, `flox_env_capture_is_deterministic`, `nix_provider_e2e`,
      `nix_env_capture_is_deterministic`, `devbox_provider_e2e`,
      `devbox_env_capture_is_deterministic`, `provisioning_runs_no_project_code`.
- [ ] 4.4 Keep the platform axis separate from the provider axis (design.md D5): the
      macOS gaps stay `cfg`-gated with their `docs/known-gaps.md` pointers and do not
      become `ProviderCapabilities` entries. Verify no macOS gap gets absorbed into a
      row capability, which would hide a published product limitation.

## 5. The synthetic row — only if task 0 said it can exist

- [ ] 5.1 Build the row per 0.3's decision. If it stays below `up`, say so in its own
      doc comment and keep it out of the lifecycle band entirely.
- [ ] 5.2 If it runs `up`: ship a real `process-compose`, pinned by hash per platform
      and architecture, and record the licence/attribution obligations that come with
      vendoring or fetching a prebuilt binary.
- [ ] 5.3 State the row's limits where they are visible: it has no dynamic loader, so
      it never exercises the `/lib` → `ld-linux` → merged-`/usr` path
      `fleet::mount::setup_merged_usr_compat` exists for. It is not evidence that a
      real toolchain runs inside the mount view.
- [ ] 5.4 macOS: a static-ELF row does not exist there. Either give the row a
      per-platform implementation or scope it to Linux and say so — one row with two
      materially different runtimes must not be described as one row.

## 6. CI, and the measurement that decides `=all`

- [ ] 6.1 Add the fast job: `DEVCROFT_TEST_PROVIDER=test cargo test --features
      test-support`, no Nix daemon. Only if group 5 produced a row.
- [ ] 6.2 Add the per-provider jobs (nix, flox, devbox) as parallel required jobs, in
      which an available-but-failing row fails the build.
      → **Corrected while implementing group 2**: this originally read "and only
      an unavailable one skips", i.e. a job selecting one row and finding it
      absent would go green. That is the trap the contract exists to close — a
      job named `integration-devbox` passing on a runner with no devbox has
      tested nothing and says otherwise. A single explicitly-selected row that
      is unavailable now **fails**; only under `=all` do individual rows skip,
      and even there a run where every row skipped fails.
- [ ] 6.3 Measure `=all` before putting it on any critical path (design.md Open
      Question 2): wall-time against the current 159s single-row baseline, and how
      many of the ~80 currently-skipped tests become runnable. If the answer is only
      "cleaner", it does not ship.
- [ ] 6.4 Keep at least the Nix row in pre-merge CI. A mount-view regression that the
      synthetic row cannot see is precisely what `add-mount-isolation`'s branch hit —
      new mount tests green, an existing devbox test regressed.

## 7. Record what the change actually did

- [ ] 7.1 Update `docs/roadmap.md` with the milestone as scoped: default developer
      suite exercises a real Nix environment; CI has independent provider-contract
      jobs. Place it after `add-mount-isolation`, before 0.3, and state that it is
      **not** a gate on cutting 0.1.0.
- [x] 7.2 If task 0 changed `shell::resolve`'s guard, update `CLAUDE.md`'s shell
      invariant to describe what the guard now is. That paragraph is currently the
      authority on it, and a stale invariant is worse than an unwritten one.
- [ ] 7.3 Record the outcome in design.md's Open Questions — including, if it turns
      out that way, "the synthetic row is not worth its cost". A change that measures
      its own premise and finds it wanting has succeeded, not failed.

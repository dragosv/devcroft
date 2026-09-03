# Tasks — Test Runtime Fixture

## 0. The spike — decide whether the cheap row can exist, before building for it

> design.md D4: two of `up`'s own invariants refuse a non-store environment
> rather than degrading under it. Everything in group 5 waits on this answer.
> The real-provider matrix (groups 1–4) does not, and must not be delayed by it.

- [ ] 0.1 Build a fixture whose `PATH` points outside `/nix/store` and call the
      real `up`. Confirm the failure is where design.md reads it: `shell::resolve`
      returning `None` at `up.rs:226`, not a weaker outcome. Record the exact
      error a developer would see.
- [ ] 0.2 Confirm the second refusal independently: a row declaring services with
      no `process-compose` in its resolved environment fails at layer `provider`
      (`up.rs:836`), and that a host-installed `process-compose` does *not*
      satisfy it (`services::resolve_in_env` ignores the host `PATH`).
- [ ] 0.3 Decide between D4's three options — generalize the store guard to "inside
      a declared provider grant", drop the synthetic row, or scope it below `up`.
      Write the measured answer into design.md before group 5 starts.
- [ ] 0.4 If 0.3 chose to generalize the guard: measure that a host shell is still
      refused afterwards, against `samples/flox-services-sample` — the case whose
      first version of this function picked `/usr/bin/dash`. A guard that admits
      the fixture *and* the host has removed the invariant rather than widened it.

## 1. The seam

- [ ] 1.1 Extract `up_with_provider(...)` from `up`'s body at the provider-selection
      boundary, leaving public `up()` as validate → build `ProviderKind` → delegate.
      Verify by diffing: no enforcement step moved out of the shared path.
- [ ] 1.2 Cover all four provider entry points, not resolution alone (design.md D1's
      table): resolution, `manifest_fingerprint`, `static_name` for rule origins, and
      `services_declared_by_flox`. Verify each is reachable from an injected row.
- [ ] 1.3 Add the non-default `test-support` feature and a `#[doc(hidden)] pub mod
      test_support`. Verify the seam is absent from a default `cargo build` —
      inspect the built binary, not just the feature flag.
- [ ] 1.4 Confirm `ProviderKind` gains no variant and `config::parse` accepts no new
      `env.provider` value. Verify with a test asserting a manifest naming a fixture
      is rejected, and that the message still distinguishes "not yet supported" from
      "out of scope by design".
- [ ] 1.5 Add `up_with_resolution(...)` for the narrower band that never needs `up`
      (hooks, `services::render_config`, keeper-direct, `spawn_keeper`). This is the
      cheap half of design.md D1 and needs none of the above — land it first if the
      seam work stalls.

## 2. The fixture contract

- [ ] 2.1 Define `ProviderFixture` — `setup` (`None` = row unavailable),
      `mutate_to_drift`, `name`, `capabilities` — plus `ProviderCapabilities`.
      Keep `Provider` unchanged (design.md D2).
- [ ] 2.2 Implement `fixture_for()`: `DEVCROFT_TEST_PROVIDER` unset → Nix flake row;
      `flox|nix|devbox|test` → that row; `all` → iterate. One selection point, not
      one per test file.
- [ ] 2.3 Implement the **no-fallback** rule: a failed setup on an explicitly
      selected row fails the run, naming `DEVCROFT_TEST_PROVIDER=test` as the
      alternative. Verify by making the default row unavailable and confirming the
      run fails rather than downgrading.
- [ ] 2.4 Implement per-row skip reporting, so `=all` ends with a legible matrix
      (`test ✓, flox ✓, nix skip(no daemon), devbox skip`). Verify a run where every
      row skipped is not reported as success.
- [ ] 2.5 Capability gating: a neutral test consults `capabilities()`, never
      `name()`. Verify with a lint or a test that greps the neutral files for
      name-branching — the rule is only real if breaking it is caught.

## 3. The Nix flake row (the default)

- [ ] 3.1 Build the row on a minimal flake — shell, coreutils, `process-compose` —
      reusing the inline-flake pattern `tests/provisioning_runs_no_project_code.rs`
      already uses, including its system double (`aarch64-darwin` vs `-linux`).
- [ ] 3.2 Verify it satisfies the realism requirement: the shell resolves out of the
      closure and not the host, and `process-compose` comes from the environment.
      Assert this in the row itself, so a future edit cannot quietly host-source it.
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
      which an available-but-failing row fails the build and only an unavailable one
      skips.
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
- [ ] 7.2 If task 0 changed `shell::resolve`'s guard, update `CLAUDE.md`'s shell
      invariant to describe what the guard now is. That paragraph is currently the
      authority on it, and a stale invariant is worse than an unwritten one.
- [ ] 7.3 Record the outcome in design.md's Open Questions — including, if it turns
      out that way, "the synthetic row is not worth its cost". A change that measures
      its own premise and finds it wanting has succeeded, not failed.

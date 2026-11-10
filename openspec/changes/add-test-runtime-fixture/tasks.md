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
      **Extended later**: the seam covered `up` but not `status`, which
      re-derived a provider from the manifest — so an injected row's
      fingerprint was honoured going in and ignored coming out, and staleness
      was unassertable for it. `status_with_provider` closes that; every row
      now answers staleness, teeth-checked by disabling the row's drift and
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
- [x] 3.3 Verify the row is skippable-but-loud: on a host with no usable Nix store,
      the default run fails with the remedy rather than skipping silently.
      → Verified 2026-11-10 by shadowing `nix` with one that fails: the
      default run panics naming the row and the reason ("a flakes-enabled
      nix is not on PATH") and the remedy (`DEVCROFT_TEST_PROVIDER=test`, or
      another row). Not simulated: a present `nix` with the daemon down,
      which would mean stopping this host's daemon. That branch goes through
      the same `Unavailable` and the same panic.

## 4. Migrate the neutral surface

> design.md Migration Plan: a file moves only once it still passes on the real
> provider it used to hardcode. A file that passes only on the synthetic row is a
> coverage regression shaped like a migration.

- [x] 4.1 Enumerate the neutral surface explicitly and record the list, starting from
      `cli_lifecycle_and_policy`, `concurrency_and_suspend`, `exec_up`, `ssh_up`,
      `lifecycle_*`, `network_isolation_e2e`. The boundary is declared, not discovered.
      → **Enumerated. 32 files invoke a provider; 7 are testing one.**
      
      *Provider contract — stay hardcoded and real (7 files, 23 tests):*
      `flox_derived_env`, `flox_env_capture_is_deterministic`,
      `nix_provider_e2e`, `nix_env_capture_is_deterministic`,
      `devbox_provider_e2e`, `devbox_env_capture_is_deterministic`,
      `provisioning_runs_no_project_code`.
      
      *Neutral surface — migrate (22 files):* `cli_lifecycle_and_policy`,
      `concurrency_and_suspend`, `concurrent_up`, `egress_proxy_e2e`,
      `exec_auto_up`, `exec_up`, `host_port_reachability`,
      `isolated_egress_e2e`, `lifecycle_down_kills_sessions`,
      `lifecycle_hooks`, `lifecycle_recreate`, `lifecycle_status`,
      `lifecycle_up`, `mount_view_e2e`, `network_isolation_e2e`,
      `network_ports_listen`, `process_tier_landlock_boundaries`, `proxy_up`,
      `shell_up`, `ssh_channels`, `ssh_up`, `symlink_escape_cli`,
      `udp_egress_denied`.
      
      *Three that do not classify cleanly, named rather than filed:*
      - `init_and_doctor_cli` (29 tests, the largest single file). `devcroft
        init` inspects a project to decide which provider to write into the
        manifest — that detection **is** provider-shaped behaviour, so part of
        this file is contract and part is neutral. Split it before migrating;
        moving it wholesale would either lose the detection coverage or drag
        provider assertions into the neutral surface.
      - `services_e2e` (8 tests). Neutral in shape, but services are flox-only,
        so it gates on `capabilities().services` and runs on one row. Worth
        migrating for the gating shape, not for matrix breadth.
      - `mount_view_e2e` and `process_tier_landlock_boundaries`. Neutral across
        providers but heavily platform-gated (Linux-only mechanisms). They
        belong to the *platform* axis, which design.md D5 keeps separate — they
        migrate, but their skips stay `cfg`-based and must not become row
        capabilities.
- [x] 4.2 Migrate them to `fixture_for()`, one file at a time, each still green on its
      original provider first.
      → **5 of 22 done**, each following the rule: verified green on the flox
      it used to hardcode *before* being allowed onto another row.
      `lifecycle_up`, `lifecycle_recreate`, `lifecycle_status`,
      `lifecycle_down_kills_sessions`, `exec_up`. All now pass on **all four rows**
      — nix, flox, devbox, and the Nix-free `test` row — which is the payoff:
      they run on a host with no Nix daemon at all.
      Two things the migration surfaced, both handled by capability rather
      than by name: `lifecycle_status` asserts `env_stale`, which the injected
      row cannot express, so that one assertion is gated on
      `capabilities().staleness`; and the per-test cost went *down* rather
      than up (measured: `lifecycle_up` 11s hardcoded-flox → 6.8s on flox via
      the fixture, 6.5s on nix).
      A third capability fell out of `exec_up`: it exec's *external commands*
      (`pwd`, `sleep`), and the Nix-free row supplies a shell and nothing
      else — a builtin does not satisfy `Command::new("pwd")`. Declared as
      `capabilities().external_utils` rather than rewritten through `sh -c`,
      which would have changed what the test asserts.
      **Blocker cleared (2026-11-08, `add-nix-free-test-row` 6.1).** On a
      Linux host with no `/nix` at all, `DEVCROFT_TEST_PROVIDER=test` runs
      every fixture test green, while the default row fails there as
      designed. So the remaining 17 files can migrate without turning a
      daemon-less host red, provided that host's job selects the row
      (group 6.1). Until each one moves, it still skips there.
      **10 of 22 (2026-11-08):** `concurrent_up`, `egress_proxy_e2e` (the
      recovery test; the `curl` test stays on flox because no row ships
      `curl`), `proxy_up`, `ssh_up`, `symlink_escape_cli` (its `up` half;
      `policy --render` needs no row). Green on nix, devbox and `test`
      under `=all`, and on the default row. `proxy_up`'s auto-up test
      skips the `test` row by `capabilities().cli_drivable`, since auto-up
      is the CLI resolving a provider from `devcroft.toml`.
      **Flox was not re-verified for this batch**, which breaks this task's
      rule as written: the host had no FloxHub login, and the flox row
      `flox install`s packages, which since flox 1.4 needs one. The old
      hardcoded tests only ran `flox init`, which does not. Re-run
      `DEVCROFT_TEST_PROVIDER=flox` on a logged-in host before calling
      these five done on flox.
      **12 of 22 (2026-11-09):** `exec_auto_up` (auto-up half gated on
      `cli_drivable`; the `--no-up` half refuses before any provider and runs
      on every row) and `shell_up`. `shell_up` found two gaps in the
      Nix-free row's shell, both in brush rather than devcroft:
      its `reedline` editor queries the cursor position on a pty and aborts
      when nothing answers (fixed in the row: `examples/test-row-sh` re-execs
      with `--input-backend=basic` on a terminal), and `trap '' TERM` is
      recorded but never installed, so no session can ignore a signal
      (declared as `capabilities().signal_traps`, false for that row only).
      Also: the row now refuses a `test-row-sh` older than its source, after
      `cargo test --test <file>` (which does not rebuild examples) ran a
      stale one without complaint. Flox again unverified, same reason.
      **15 of 22 (2026-11-09):** `concurrency_and_suspend` (two sandboxes on
      one row, via `common::another_of`), `lifecycle_hooks`, `ssh_channels`
      (all but `rsync`, which needs rsync inside the sandbox) and
      `cli_lifecycle_and_policy` (the three tests whose subject is
      `devcroft up` gate on `cli_drivable`; its provider-grants test now
      reads the grant from `Meta` rather than expecting `/nix/store` and
      `provider:flox`). Three things the batch found:
      - **A product bug.** `up` cleared `FD_CLOEXEC` on the keeper's
        listeners in the caller and leaked them, so parallel in-process
        `up`s handed every sandbox's listeners to every subprocess, and
        `ps` hung on a removed sandbox's still-accepting socket. Fixed in
        `src/lifecycle/up.rs`, asserted by `tests/up_closes_its_listeners.rs`.
      - **The Nix-free row's grant was hidden by the baseline.** Its
        environment lived under `/tmp`, which the baseline grants, so the
        row could not have caught its own grant going missing. It now lives
        under `CARGO_TARGET_TMPDIR`, outside every baseline grant, as
        `/nix/store` is.
      - **The row leaked its environment dir** (no `Drop`; a ~146 MB shell
        copy each): 72 directories, 9.9 GB in `/tmp`. It now has a `Drop`
        and hard-links the shell.
      **22 of 22 (2026-11-10).** The probe-bound files below were migrated
      by giving rows the probes rather than rewriting them: nix, flox and
      devbox now ship `python3` and `curl` (`capabilities().network_probes`;
      false for the Nix-free row, and for swift until measured there), and
      the flox lock was regenerated. `network_isolation_e2e`'s two sandboxes
      of one project needed `ProviderFixture::bring_up_with(&Manifest)`.
      Two of the five gained the control their own comments described:
      `host_port_reachability` now asserts the server answered inside before
      reading "unreachable from the host" as isolation, and
      `udp_egress_denied` skips unless the host itself gets a DNS reply,
      so an offline runner no longer passes it. Teeth-checked: with
      `default = "allow"` the UDP test fails on `LEAK 61`. Only
      `ssh_channels`' rsync test stays on flox, since it needs rsync on the
      host as well. Verified: default row 749 passed, `=all` over the 26
      row-fixture files 58 passed, every matrix line `nix ok, flox ok,
      devbox ok`.
      **Probe-bound at 17 of 22, since migrated (5 files + 2 tests):**
      `network_isolation_e2e`, `isolated_egress_e2e`,
      `host_port_reachability`, `network_ports_listen`, `udp_egress_denied`,
      plus `egress_proxy_e2e`'s curl test and `ssh_channels`' rsync test.
      Their claims are neutral but each needs `python3`, `curl` or `rsync`
      inside the sandbox, which no row ships; rewriting the probes in shell
      would weaken them (the UDP test needs a reply, not just a send).
      Decided: rows ship the probes (see 22 of 22 above).
      **17 of 22 (2026-11-10):** `process_tier_landlock_boundaries` (all
      three tests, green on nix, flox, devbox; the Nix-free row skips by a
      new capability, `boundary_probes`: it has no bash or `cat`, and
      brush's `kill` fails without naming the errno, so a failure there is
      no evidence of a refusal). Each `/dev/tcp` test now first asserts
      bash runs in the sandbox — the false pass its own comment warned
      about, unguarded until now; on the Nix-free row the guard fired
      before the capability existed. `mount_view_e2e` is **not a
      migration**: neither test goes through `up`, one is a real `cargo
      build` from devbox's closure (provider content no row ships), the
      other uses the host's `rustc` and no provider at all. Counted as
      settled, left as is.
      **The flox row no longer resolves (2026-11-09).** It copies a
      checked-in, already-locked environment (`tests/fixtures/flox-row`,
      bash + coreutils, all four systems) instead of `flox install`ing:
      activating a locked environment needs no FloxHub login, resolving one
      does (measured, flox 1.14 — `up` and `exec` ran on a locked copy with
      no login). The row refuses a lock that no longer matches its manifest,
      since flox would otherwise re-resolve silently on a logged-in host.
      The lock is regenerated by `scripts/lock-flox-test-row.sh`, which is
      the one step needing a login. A project's *first* lock keeps its own
      test, `tests/flox_fresh_resolution.rs`, which runs when flox is
      selected and **fails rather than skips** without a login. Lock
      generated 2026-11-10 (flox 1.14.0, which also rewrote the manifest to
      `schema-version = "1.14.0"` with `outputs = "all"` — so the row needs a
      flox that reads that schema). **Flox re-verified:** all 20 files on the
      row fixture green under `=flox` (no flox skip), and
      `flox_fresh_resolution` green with a login. This closes the
      "flox unverified" caveat on every batch above.
- [x] 4.3 Leave the 7 provider-contract files hardcoded and real:
      `flox_derived_env`, `flox_env_capture_is_deterministic`, `nix_provider_e2e`,
      `nix_env_capture_is_deterministic`, `devbox_provider_e2e`,
      `devbox_env_capture_is_deterministic`, `provisioning_runs_no_project_code`.
      → Done: all seven still build their own provider projects, and none
      uses `common` (checked 2026-11-10).
- [x] 4.4 Keep the platform axis separate from the provider axis (design.md D5): the
      macOS gaps stay `cfg`-gated with their `docs/known-gaps.md` pointers and do not
      become `ProviderCapabilities` entries. Verify no macOS gap gets absorbed into a
      row capability, which would hide a published product limitation.
      → Done (2026-11-10): the eight capabilities (`services`,
      `activation_hook`, `external_utils`, `cli_drivable`, `staleness`,
      `signal_traps`, `boundary_probes`, `network_probes`) each describe a
      row's tooling or its provider, none a platform. The macOS gaps the
      migration touched stayed `cfg`-gated: `network_ports_listen`'s per-port
      listen scoping and `process_tier_landlock_boundaries`' `/proc` read.

## 5. The synthetic row — only if task 0 said it can exist

- [x] 5.1 Build the row per 0.3's decision. If it stays below `up`, say so in its own
      doc comment and keep it out of the lifecycle band entirely.
      → Done in `add-nix-free-test-row` (archived 2026-11-08): the row runs
      `up` through the seam, under the guard 0.3 generalized.
- [x] 5.2 If it runs `up`: ship a real `process-compose`, pinned by hash per platform
      and architecture, and record the licence/attribution obligations that come with
      vendoring or fetching a prebuilt binary.
      → Decided against in `add-nix-free-test-row` 4.1: no process-compose
      (no Rust reimplementation to vendor the cheap way), so no pinned
      binary and no licence obligation. `capabilities().services` is false
      for the row and services tests skip it by capability.
- [x] 5.3 State the row's limits where they are visible: it has no dynamic loader, so
      it never exercises the `/lib` → `ld-linux` → merged-`/usr` path
      `fleet::mount::setup_merged_usr_compat` exists for. It is not evidence that a
      real toolchain runs inside the mount view.
      → Done 2026-11-10, on `NixFreeRow` itself. The archived change
      recorded this as done in the row's comments, but no such comment
      exists, and the statement had been written for the planned static
      BusyBox, which N6 replaced with `brush` — so it was also false: the row's shell is now *host*-linked (host
      `ld-linux` + glibc, readable through the keeper's baseline
      `KEEPER_SYSTEM_READ` grant), not loader-free. The limit is the same,
      by the opposite mechanism, and the doc says so.
- [x] 5.4 macOS: a static-ELF row does not exist there. Either give the row a
      per-platform implementation or scope it to Linux and say so — one row with two
      materially different runtimes must not be described as one row.
      → Superseded premise: the row is no longer a static ELF. It is one
      Rust source compiled natively per platform, so one row, not two
      runtimes under one name. Its macOS run since the move to `brush` is
      not recorded; noted on `NixFreeRow`.

## 6. CI, and the measurement that decides `=all`

- [x] 6.1 Add the fast job: `DEVCROFT_TEST_PROVIDER=test cargo test --features
      test-support`, no Nix daemon. Only if group 5 produced a row.
      → Already in `.github/workflows/ci.yml` (`test` job, ubuntu leg:
      `DEVCROFT_TEST_PROVIDER=test cargo test --features test-support`).
- [x] 6.2 Add the per-provider jobs (nix, flox, devbox) as parallel required jobs, in
      which an available-but-failing row fails the build.
      → **Corrected while implementing group 2**: this originally read "and only
      an unavailable one skips", i.e. a job selecting one row and finding it
      absent would go green. That is the trap the contract exists to close — a
      job named `integration-devbox` passing on a runner with no devbox has
      tested nothing and says otherwise. A single explicitly-selected row that
      is unavailable now **fails**; only under `=all` do individual rows skip,
      and even there a run where every row skipped fails.
      → Already in ci.yml (`e2e` matrix; flox, nix and devbox blocking,
      devenv continue-on-error for its recorded nix-evaluator failure).
      2026-11-10: the flox leg now passes `FLOX_FLOXHUB_TOKEN` from
      secrets, for `tests/flox_fresh_resolution.rs`, which fails without a
      login by design. **The secret has to be created by the owner**; until
      then, and on fork PRs, that leg is red.
- [ ] 6.3 Measure `=all` before putting it on any critical path (design.md Open
      Question 2): wall-time against the current 159s single-row baseline, and how
      many of the ~80 currently-skipped tests become runnable. If the answer is only
      "cleaner", it does not ship.
- [x] 6.4 Keep at least the Nix row in pre-merge CI. A mount-view regression that the
      synthetic row cannot see is precisely what `add-mount-isolation`'s branch hit —
      new mount tests green, an existing devbox test regressed.
      → Already: the `e2e (nix)` leg runs on `pull_request` and is blocking.

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

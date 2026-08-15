# Tasks: add-nix-provider

## 1. Extract shared capture machinery

- [x] 1.1 Move `canonical_base_env`, `changed_env`, `unset_env`,
      `store_grants`, and the NUL-separated `env -0` parsing out of
      `provider/flox.rs` into a `provider/capture.rs` module private to
      `provider/`, with `FloxProvider` consuming it; existing flox tests
      and the integration suite stay green with zero behavior change
- [x] 1.2 Unit-test the shared capture against a synthetic activation
      dump: changed keys, unset keys, store-root extraction from values
      containing store paths mid-string

## 2. Config and validation surface

- [x] 2.1 `provider::validate`: move `nix`/`flake`/`flakes` out of
      `NOT_YET_SUPPORTED`; accept them; update the rejection-message
      tests (`nix_flakes_reports_planned_closure_tier` becomes an
      acceptance test)
- [x] 2.2 Config parse: normalize `flake`/`flakes` to `nix` at parse
      time so the resolved config, `status`, and policy origins only
      ever see `nix`; default stays `flox`; unit tests for canonical
      name, both aliases, and the unchanged default
- [x] 2.3 Provider dispatch: replace `lifecycle::up`'s hard-wired
      `FloxProvider` with a two-variant enum keyed off the validated
      provider name; staleness fingerprinting dispatches the same way

## 3. NixProvider

- [x] 3.1 Preconditions in order (design decision 5): `flake.nix`
      present (hint `nix flake init`), `nix` resolved on the ambient
      PATH via `paths::resolve_on_path` (hint `devcroft doctor`),
      `flake.lock` present (hint `nix flake lock`), then one
      `nix flake metadata --no-update-lock-file` probe distinguishing
      flakes-disabled / daemon-unreachable / lock-not-covering-inputs;
      all layer `provider`, exit 3
- [x] 3.2 Activation capture: `nix develop <root> --no-update-lock-file
      --command` writing `env -0` output to a temp file (shellHook
      stdout chatter must not corrupt the capture — test with a flake
      whose shellHook prints), diffed through the shared capture module;
      never pass `--impure`
- [x] 3.3 Staleness: fingerprint `flake.nix` + `flake.lock` under the
      same contract as flox's `manifest_fingerprint`/`is_stale`
- [x] 3.4 Store grants carry origin `provider:nix`; `policy --render`
      and `why` show them; assert no write grant can originate from the
      provider (existing "Provider does not weaken the sandbox"
      requirement, now exercised for a second provider). Found along the
      way: `policy --render`/`why` never showed *any* provider's store
      grants before this — `Origin::Provider` existed since MVP but had
      no caller. Fixed for flox and nix alike by having `up` persist
      `resolution.read_only_grants` into `lifecycle::state::Meta`, and
      `policy --render`/`why` read it back and merge it via the new
      `CompiledPolicy::with_provider_grants`. Also found and fixed a
      related pre-existing race while testing this: `write_meta` used a
      non-atomic `std::fs::write`, and `ps` (which reads every sandbox's
      `meta.json`, including ones mid-`up` elsewhere) could observe a
      truncated file; now writes via temp file + rename.

## 4. CLI integration

- [x] 4.1 `doctor`: nix provider check — binary presence and a flakes
      probe, FAIL naming the `experimental-features` fix when flake
      commands are rejected. nix's mere *absence* is `[WARN]`, not
      `[FAIL]` (unlike flox, which every host needs, nix is an
      alternative — most hosts won't have it and shouldn't fail doctor
      over that); only a broken nix (present, flakes disabled) fails.
- [x] 4.2 `init`: detect `flake.nix` (no `.flox/`) → `provider = "nix"`,
      printing `nix flake lock` as the next step when the lock is
      missing; both present → flox wins with a one-line note; toolchain
      pin advice suppressed when either environment exists
- [x] 4.3 Error messages that hard-code flox (`ProviderError::
      MissingBinary`, `NoEnvironment`, `Unknown`'s "supports `flox`")
      become provider-aware

## 5. Integration tests

- [x] 5.1 Test fixture: a minimal committed flake (pinned nixpkgs input,
      one small package in the dev shell, `flake.lock` checked in);
      tests self-skip when `nix` with flakes is missing from PATH, same
      pattern as the existing flox/nono skips. (Fixture lives inline in
      each test file as a const, same as `provider::nix`'s own unit
      tests — not a checked-in flake.lock, since one was needed per
      fixture instance to stay independent across test files; `nix flake
      lock` runs as part of each test's setup and the whole test skips
      if that fails, e.g. no network for nixpkgs.)
- [x] 5.2 `up`/`exec` end-to-end (`tests/nix_provider_e2e.rs`): tool from
      the dev shell visible in a session; runs under the default
      `network.default = "deny"`. Env diff determinism under a polluted
      invoking shell (`tests/nix_env_capture_is_deterministic.rs`,
      mirroring the existing flox version) — both share
      `provider::capture`'s fixed-baseline mechanism, so this is the
      same guarantee exercised for a second provider, not a new one.
- [x] 5.3 Failure paths (`tests/nix_provider_e2e.rs`): missing
      `flake.nix` and missing `flake.lock` each fail `up` at exit code 3
      with the specified hint, through the real CLI. ("input absent from
      the lock" is covered at the provider-unit level instead
      (`classify_metadata_failure` tests in `provider/nix.rs`) rather
      than duplicated as a full CLI e2e case — reaching it through a
      real `nix flake metadata` call requires deliberately desyncing a
      flake input from its lock, which is awkward to construct
      reliably and the classification logic is what's actually being
      tested.)
- [x] 5.4 Staleness (`tests/nix_provider_e2e.rs`): touch `flake.nix`,
      `status` reports stale naming "flake" (not flox's wording —
      `print_status` needed to become provider-aware here too, a small
      gap in task 4.3's scope found while writing this test). A plain
      `up` on a stale-but-healthy keeper stays idempotent by design
      (only `--recreate` re-resolves); asserted explicitly so the test
      doesn't imply otherwise.

## 6. Docs and sample

- [x] 6.1 `samples/nix-flake-sample`: standalone example project (own
      `[workspace]` table, own README) mirroring the flox samples,
      demonstrating a flake-backed sandbox — without a project-local
      `TMPDIR` redirect (see docs/ssh-validation.md on the 104-byte
      socket-path limit). Built and verified end to end against a real
      `nono`+`nix` sandbox on this host: `up`, `exec -- cargo build`
      (fully offline), the built binary, `ssh`, `status`,
      `policy --render` (shows `/nix/store` with `provider:nix`), and
      `why`. Found and fixed two real problems along the way, documented
      in the sample's own README: `builtins.currentSystem` isn't
      available under nix's pure evaluation (the same
      static-systems-list fix `provider::nix`'s test fixtures already
      use), and nixpkgs' `cargo` trusts no CA bundle by default
      (`pkgs.cacert` + `SSL_CERT_FILE`/`NIX_SSL_CERT_FILE`, the standard
      nix devShell fix, unrelated to devcroft).
- [x] 6.2 README + CLAUDE.md: `openspec validate --all` count updated
      (3 → 4 passed); "only `add-mvp-core` is actually implemented"
      corrected to include `add-nix-provider`; README's Status section
      gained a "Post-MVP" paragraph. `docs/decisions.md` needed no
      change — it only ever listed nix flakes as a closure-tier design
      *example*, never claimed it unsupported, so there was no stale
      rejection copy to update there. `add-mise-provider`'s stated
      dependency ("provider trait proven on at least one additional
      closure-tier provider") is now satisfied.

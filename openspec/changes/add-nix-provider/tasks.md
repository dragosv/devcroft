# Tasks: add-nix-provider

## 1. Extract shared capture machinery

- [ ] 1.1 Move `canonical_base_env`, `changed_env`, `unset_env`,
      `store_grants`, and the NUL-separated `env -0` parsing out of
      `provider/flox.rs` into a `provider/capture.rs` module private to
      `provider/`, with `FloxProvider` consuming it; existing flox tests
      and the integration suite stay green with zero behavior change
- [ ] 1.2 Unit-test the shared capture against a synthetic activation
      dump: changed keys, unset keys, store-root extraction from values
      containing store paths mid-string

## 2. Config and validation surface

- [ ] 2.1 `provider::validate`: move `nix`/`flake`/`flakes` out of
      `NOT_YET_SUPPORTED`; accept them; update the rejection-message
      tests (`nix_flakes_reports_planned_closure_tier` becomes an
      acceptance test)
- [ ] 2.2 Config parse: normalize `flake`/`flakes` to `nix` at parse
      time so the resolved config, `status`, and policy origins only
      ever see `nix`; default stays `flox`; unit tests for canonical
      name, both aliases, and the unchanged default
- [ ] 2.3 Provider dispatch: replace `lifecycle::up`'s hard-wired
      `FloxProvider` with a two-variant enum keyed off the validated
      provider name; staleness fingerprinting dispatches the same way

## 3. NixProvider

- [ ] 3.1 Preconditions in order (design decision 5): `flake.nix`
      present (hint `nix flake init`), `nix` resolved on the ambient
      PATH via `paths::resolve_on_path` (hint `devcroft doctor`),
      `flake.lock` present (hint `nix flake lock`), then one
      `nix flake metadata --no-update-lock-file` probe distinguishing
      flakes-disabled / daemon-unreachable / lock-not-covering-inputs;
      all layer `provider`, exit 3
- [ ] 3.2 Activation capture: `nix develop <root> --no-update-lock-file
      --command` writing `env -0` output to a temp file (shellHook
      stdout chatter must not corrupt the capture — test with a flake
      whose shellHook prints), diffed through the shared capture module;
      never pass `--impure`
- [ ] 3.3 Staleness: fingerprint `flake.nix` + `flake.lock` under the
      same contract as flox's `manifest_fingerprint`/`is_stale`
- [ ] 3.4 Store grants carry origin `provider:nix`; `policy --render`
      and `why` show them; assert no write grant can originate from the
      provider (existing "Provider does not weaken the sandbox"
      requirement, now exercised for a second provider)

## 4. CLI integration

- [ ] 4.1 `doctor`: nix provider check — binary presence and a flakes
      probe, FAIL naming the `experimental-features` fix when flake
      commands are rejected
- [ ] 4.2 `init`: detect `flake.nix` (no `.flox/`) → `provider = "nix"`,
      printing `nix flake lock` as the next step when the lock is
      missing; both present → flox wins with a one-line note; toolchain
      pin advice suppressed when either environment exists
- [ ] 4.3 Error messages that hard-code flox (`ProviderError::
      MissingBinary`, `NoEnvironment`, `Unknown`'s "supports `flox`")
      become provider-aware

## 5. Integration tests

- [ ] 5.1 Test fixture: a minimal committed flake (pinned nixpkgs input,
      one small package in the dev shell, `flake.lock` checked in);
      tests self-skip when `nix` with flakes is missing from PATH, same
      pattern as the existing flox/nono skips
- [ ] 5.2 `up`/`exec` end-to-end: tool from the dev shell visible in a
      session; runs under `network.default = "deny"`; env diff identical
      when `up` is invoked with a polluted shell environment
- [ ] 5.3 Failure paths: missing `flake.nix`, missing `flake.lock`, and
      an input absent from the lock each fail `up` at layer `provider`
      with the specified hint and exit code 3
- [ ] 5.4 Staleness: touch `flake.nix`, `status` reports stale, `up`
      prints the `--recreate` notice

## 6. Docs and sample

- [ ] 6.1 `samples/nix-flake-sample`: standalone example project (own
      `[workspace]` table, own README) mirroring the flox samples,
      demonstrating a flake-backed sandbox — without a project-local
      `TMPDIR` redirect (see docs/ssh-validation.md on the 104-byte
      socket-path limit)
- [ ] 6.2 README + docs/decisions.md: nix flakes moves from "not yet
      supported" to supported (closure tier); update the provider
      rejection copy quoted in docs; note `add-mise-provider`'s
      dependency is now satisfied
